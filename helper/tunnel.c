#define _POSIX_C_SOURCE 200809L
/* Persistent RH02 outbound TCP multiplexer for local and SOCKS5 forwards. */
#include "tunnel.h"
#include "mux.h"
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <unistd.h>

#define TUN_MAX 64
#define TUN_CHUNK 16384u
#define TUN_OUT_HIGH (256u * 1024u)

typedef struct {
    unsigned char header[RH_MUX_HEADER], *body;
    size_t at, size;
    int complete;
} rx_frame;

typedef struct {
    unsigned char *data;
    size_t len, cap, off;
} byte_buf;

typedef struct {
    uint64_t id;
    int fd;
    int connecting;
    int rx_eof;
    int tx_eof;
    int last_err;
    struct addrinfo *ai_head;
    struct addrinfo *ai_cur;
    byte_buf tx;
} tun_conn;

static int nonblock(int fd) {
    int flags = fcntl(fd, F_GETFL);
    return flags >= 0 && fcntl(fd, F_SETFL, flags | O_NONBLOCK) >= 0;
}

static int buf_append(byte_buf *b, const void *p, size_t n) {
    if (!n) return 1;
    if (b->off == b->len) { b->off = 0; b->len = 0; }
    if (b->off > 0 && b->len + n > b->cap) {
        size_t keep = b->len - b->off;
        memmove(b->data, b->data + b->off, keep);
        b->len = keep;
        b->off = 0;
    }
    if (b->len + n > b->cap) {
        size_t cap = b->cap ? b->cap : 4096;
        while (cap < b->len + n) {
            if (cap > (size_t)RH_LIMIT) return 0;
            cap *= 2;
        }
        unsigned char *next = realloc(b->data, cap);
        if (!next) return 0;
        b->data = next;
        b->cap = cap;
    }
    memcpy(b->data + b->len, p, n);
    b->len += n;
    return 1;
}

static void buf_reset(byte_buf *b) {
    free(b->data);
    memset(b, 0, sizeof *b);
}

static int queue_reply(byte_buf *out, uint64_t id, uint32_t kind, const void *body, size_t len) {
    unsigned char header[RH_MUX_HEADER];
    rh_mux_header(header, id, kind, len);
    return buf_append(out, header, sizeof header) && buf_append(out, body, len);
}

static int queue_open_err(byte_buf *out, uint64_t id, const char *msg, int refused) {
    rh_buf b = {0};
    rh_byte(&b, 1);
    rh_text(&b, msg);
    rh_byte(&b, refused ? 1u : 0u);
    int ok = !b.failed && queue_reply(out, id, RH_TCP_OPEN, b.data, b.size);
    free(b.data);
    return ok;
}

static void conn_close(tun_conn *c) {
    if (c->fd >= 0) close(c->fd);
    if (c->ai_head) freeaddrinfo(c->ai_head);
    buf_reset(&c->tx);
    memset(c, 0, sizeof *c);
    c->fd = -1;
}

static void emit_close(byte_buf *out, tun_conn *c, unsigned char full) {
    unsigned char flag = full ? 0u : 1u;
    (void)queue_reply(out, c->id, RH_TCP_CLOSE, &flag, 1);
    if (full) conn_close(c);
}

static int start_connect(tun_conn *c, byte_buf *out) {
    while (c->ai_cur) {
        struct addrinfo *ai = c->ai_cur;
        int fd = socket(ai->ai_family, ai->ai_socktype, ai->ai_protocol);
        if (fd < 0) {
            c->last_err = errno;
            c->ai_cur = ai->ai_next;
            continue;
        }
        if (!nonblock(fd)) {
            c->last_err = errno;
            close(fd);
            c->ai_cur = ai->ai_next;
            continue;
        }
        int one = 1;
        (void)setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof one);
        if (connect(fd, ai->ai_addr, ai->ai_addrlen) == 0) {
            c->fd = fd;
            c->connecting = 0;
            freeaddrinfo(c->ai_head);
            c->ai_head = NULL;
            c->ai_cur = NULL;
            unsigned char ok = 0;
            return queue_reply(out, c->id, RH_TCP_OPEN, &ok, 1);
        }
        if (errno == EINPROGRESS) {
            c->fd = fd;
            c->connecting = 1;
            return 1;
        }
        c->last_err = errno;
        close(fd);
        c->ai_cur = ai->ai_next;
    }
    int err = c->last_err ? c->last_err : ECONNREFUSED;
    int refused = err == ECONNREFUSED;
    (void)queue_open_err(out, c->id, strerror(err), refused);
    conn_close(c);
    return 1;
}

static tun_conn *find_conn(tun_conn conns[TUN_MAX], uint64_t id) {
    for (unsigned i = 0; i < TUN_MAX; i++) {
        if (conns[i].id == id) return &conns[i];
    }
    return NULL;
}

static int handle_open(tun_conn conns[TUN_MAX], byte_buf *out, uint64_t id, const unsigned char *body, size_t size) {
    if (find_conn(conns, id)) {
        return queue_open_err(out, id, "duplicate tunnel stream id", 0);
    }
    rh_reader r = {body, size, 0, 0};
    char *host = rh_read_text(&r);
    uint32_t port = rh_read_int(&r);
    if (!rh_done(&r) || !host || !*host || port == 0 || port > 65535u) {
        free(host);
        return queue_open_err(out, id, "invalid TCP open destination", 0);
    }
    tun_conn *slot = NULL;
    for (unsigned i = 0; i < TUN_MAX; i++) {
        if (conns[i].id == 0) { slot = &conns[i]; break; }
    }
    if (!slot) {
        free(host);
        return queue_open_err(out, id, "remote tunnel stream capacity is full", 0);
    }
    char service[16];
    snprintf(service, sizeof service, "%u", (unsigned)port);
    struct addrinfo hints;
    memset(&hints, 0, sizeof hints);
    hints.ai_socktype = SOCK_STREAM;
    struct addrinfo *res = NULL;
    int gai = getaddrinfo(host, service, &hints, &res);
    free(host);
    if (gai != 0 || !res) {
        return queue_open_err(out, id, gai_strerror(gai), 0);
    }
    memset(slot, 0, sizeof *slot);
    slot->id = id;
    slot->fd = -1;
    slot->ai_head = res;
    slot->ai_cur = res;
    return start_connect(slot, out);
}

static int recv_frame(int fd, rx_frame *f) {
    int header = f->at < RH_MUX_HEADER;
    unsigned char *next = header ? f->header + f->at : f->body + f->at - RH_MUX_HEADER;
    size_t left = header ? RH_MUX_HEADER - f->at : RH_MUX_HEADER + f->size - f->at;
    ssize_t n = read(fd, next, left);
    if (n == 0) return 0;
    if (n < 0) return errno == EINTR || errno == EAGAIN ? 1 : -1;
    f->at += (size_t)n;
    if (header && f->at == RH_MUX_HEADER) {
        if (memcmp(f->header, "RH02", 4) != 0) return -1;
        f->size = rh_u32(f->header + 16);
        if (f->size > RH_LIMIT) return -1;
        if (f->size && !(f->body = malloc(f->size))) return -1;
    }
    f->complete = f->at >= RH_MUX_HEADER && f->at == RH_MUX_HEADER + f->size;
    return 1;
}

static void reset_rx(rx_frame *f) {
    free(f->body);
    memset(f, 0, sizeof *f);
}

static void handle_frame(tun_conn conns[TUN_MAX], byte_buf *out, const rx_frame *f) {
    uint64_t id = rh_u64(f->header + 4);
    uint32_t kind = rh_u32(f->header + 12);
    if (kind == RH_TCP_OPEN) {
        (void)handle_open(conns, out, id, f->body, f->size);
        return;
    }
    tun_conn *c = find_conn(conns, id);
    if (!c) return;
    if (kind == RH_TCP_DATA) {
        if (!c->tx_eof && f->size > 0 && !buf_append(&c->tx, f->body, f->size)) {
            emit_close(out, c, 1);
        }
        return;
    }
    if (kind == RH_TCP_CLOSE) {
        unsigned full = f->size == 0 || f->body[0] == 0;
        if (full) {
            conn_close(c);
            return;
        }
        c->tx_eof = 1;
        if (!c->connecting && c->tx.off == c->tx.len) {
            if (c->rx_eof) emit_close(out, c, 1);
            else (void)shutdown(c->fd, SHUT_WR);
        }
    }
}

int rh_tunnel_run(int in_fd, int out_fd) {
    signal(SIGPIPE, SIG_IGN);
    if (!nonblock(in_fd) || !nonblock(out_fd)) return 255;
    tun_conn conns[TUN_MAX];
    for (unsigned i = 0; i < TUN_MAX; i++) {
        memset(&conns[i], 0, sizeof conns[i]);
        conns[i].fd = -1;
    }
    rx_frame rx = {0};
    byte_buf out = {0};
    unsigned char buf[TUN_CHUNK];
    int rc = 0;
    for (;;) {
        if (rx.complete) {
            handle_frame(conns, &out, &rx);
            reset_rx(&rx);
        }
        struct pollfd fds[TUN_MAX + 2];
        int map[TUN_MAX];
        nfds_t nfds = 0;
        int pause_rx = out.len - out.off > TUN_OUT_HIGH;
        fds[nfds++] = (struct pollfd){in_fd, POLLIN, 0};
        fds[nfds++] = (struct pollfd){out_fd, out.len > out.off ? POLLOUT : 0, 0};
        for (unsigned i = 0; i < TUN_MAX; i++) {
            if (conns[i].id == 0 || conns[i].fd < 0) continue;
            short ev = 0;
            if (conns[i].connecting) {
                ev = POLLOUT;
            } else {
                if (!conns[i].rx_eof && !pause_rx) ev |= POLLIN;
                if (conns[i].tx.len > conns[i].tx.off) ev |= POLLOUT;
            }
            map[nfds - 2] = (int)i;
            fds[nfds++] = (struct pollfd){conns[i].fd, ev, 0};
        }
        if (poll(fds, nfds, -1) < 0) {
            if (errno == EINTR) continue;
            rc = 255;
            break;
        }
        if (fds[0].revents & (POLLIN | POLLHUP)) {
            int r = recv_frame(in_fd, &rx);
            if (r <= 0) { rc = r < 0 ? 255 : 0; break; }
        }
        if (fds[0].revents & (POLLERR | POLLNVAL)) { rc = 255; break; }
        if (fds[1].revents & POLLOUT && out.len > out.off) {
            ssize_t w = write(out_fd, out.data + out.off, out.len - out.off);
            if (w > 0) {
                out.off += (size_t)w;
                if (out.off == out.len) { out.off = 0; out.len = 0; }
            } else if (w < 0 && errno != EINTR && errno != EAGAIN) {
                rc = 255;
                break;
            }
        }
        if (fds[1].revents & (POLLHUP | POLLERR | POLLNVAL)) { rc = 255; break; }
        for (nfds_t k = 2; k < nfds; k++) {
            tun_conn *c = &conns[map[k - 2]];
            short rev = fds[k].revents;
            if (!rev || c->id == 0 || c->fd < 0) continue;
            if (c->connecting) {
                int err = 0;
                socklen_t elen = sizeof err;
                if (getsockopt(c->fd, SOL_SOCKET, SO_ERROR, &err, &elen) < 0) err = errno;
                if (err == 0) {
                    c->connecting = 0;
                    freeaddrinfo(c->ai_head);
                    c->ai_head = NULL;
                    c->ai_cur = NULL;
                    unsigned char ok = 0;
                    (void)queue_reply(&out, c->id, RH_TCP_OPEN, &ok, 1);
                    if (c->tx_eof && c->tx.len == c->tx.off) (void)shutdown(c->fd, SHUT_WR);
                } else {
                    c->last_err = err;
                    close(c->fd);
                    c->fd = -1;
                    c->ai_cur = c->ai_cur ? c->ai_cur->ai_next : NULL;
                    (void)start_connect(c, &out);
                }
                continue;
            }
            if (rev & POLLOUT && c->tx.len > c->tx.off) {
                ssize_t w = write(c->fd, c->tx.data + c->tx.off, c->tx.len - c->tx.off);
                if (w > 0) {
                    c->tx.off += (size_t)w;
                    if (c->tx.off == c->tx.len) {
                        c->tx.off = 0;
                        c->tx.len = 0;
                        if (c->tx_eof) {
                            if (c->rx_eof) { emit_close(&out, c, 1); continue; }
                            (void)shutdown(c->fd, SHUT_WR);
                        }
                    }
                } else if (w < 0 && errno != EINTR && errno != EAGAIN) {
                    emit_close(&out, c, 1);
                    continue;
                }
            }
            if ((rev & (POLLIN | POLLHUP)) && !c->rx_eof) {
                ssize_t n = read(c->fd, buf, sizeof buf);
                if (n > 0) {
                    if (!queue_reply(&out, c->id, RH_TCP_DATA, buf, (size_t)n)) {
                        emit_close(&out, c, 1);
                        continue;
                    }
                } else if (n == 0) {
                    c->rx_eof = 1;
                    if (c->tx_eof && c->tx.off == c->tx.len) {
                        emit_close(&out, c, 1);
                        continue;
                    }
                    emit_close(&out, c, 0);
                } else if (errno != EINTR && errno != EAGAIN) {
                    emit_close(&out, c, 1);
                    continue;
                }
            }
            if (rev & (POLLERR | POLLNVAL)) {
                emit_close(&out, c, 1);
            }
        }
    }
    reset_rx(&rx);
    buf_reset(&out);
    for (unsigned i = 0; i < TUN_MAX; i++) {
        if (conns[i].id) conn_close(&conns[i]);
    }
    return rc;
}
