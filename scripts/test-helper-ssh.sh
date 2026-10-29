#!/bin/sh
# Disposable real-SSH plumbing. Acceptance remains in the Rust live target.
set -eu
cd "$(dirname "$0")/.."
: "${RHOST_BIN:?set RHOST_BIN to an explicit binary}"
: "${RHOST_HELPER_BIN:?set RHOST_HELPER_BIN to an exact native candidate}"
test "$(uname -s)" = Linux || { echo 'temporary sshd fixture requires Linux' >&2; exit 2; }
scratch=$(mktemp -d /tmp/rhost-sshd-XXXXXXXX)
server_pid=
cleanup() {
    if [ -n "$server_pid" ]; then kill "$server_pid" 2>/dev/null || :; wait "$server_pid" 2>/dev/null || :; fi
    rm -rf -- "$scratch"
}
trap cleanup EXIT HUP INT TERM
chmod 700 "$scratch"
ssh-keygen -q -t ed25519 -N '' -f "$scratch/server"
ssh-keygen -q -t ed25519 -N '' -f "$scratch/client"
cp "$scratch/client.pub" "$scratch/authorized_keys"
chmod 600 "$scratch/authorized_keys"
account=$(id -un)
attempt=0
while :; do
    attempt=$((attempt + 1))
    test "$attempt" -le 20 || { echo 'cannot reserve disposable sshd port' >&2; exit 1; }
    random=$(od -An -N2 -tu2 /dev/urandom | tr -d ' ')
    port=$((20000 + random % 40000))
    cat > "$scratch/sshd_config" <<EOF
Port $port
ListenAddress 127.0.0.1
HostKey $scratch/server
PidFile $scratch/sshd.pid
AuthorizedKeysFile $scratch/authorized_keys
StrictModes yes
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
UsePAM no
AllowUsers $account
LogLevel ERROR
EOF
    /usr/sbin/sshd -D -f "$scratch/sshd_config" -E "$scratch/server.log" &
    server_pid=$!
    sleep 0.2
    if kill -0 "$server_pid" 2>/dev/null; then break; fi
    wait "$server_pid" 2>/dev/null || :
    server_pid=
done
printf '[127.0.0.1]:%s %s\n' "$port" "$(cat "$scratch/server.pub")" > "$scratch/known_hosts"
cat > "$scratch/ssh_config" <<EOF
Host helper-fixture
  HostName 127.0.0.1
  Port $port
  User $account
  IdentityFile $scratch/client
  IdentitiesOnly yes
  BatchMode yes
  StrictHostKeyChecking yes
  UserKnownHostsFile $scratch/known_hosts
  GlobalKnownHostsFile /dev/null
  LogLevel ERROR
EOF
mkdir "$scratch/bin"
cat > "$scratch/bin/ssh" <<'EOF'
#!/bin/sh
exec /usr/bin/ssh -F "$RHOST_FIXTURE_CONFIG" "$@"
EOF
chmod 700 "$scratch/bin/ssh"
export RHOST_FIXTURE_CONFIG="$scratch/ssh_config"
export PATH="$scratch/bin:$PATH"
export RHOST_TEST_HOST=helper-fixture
ssh -- "$RHOST_TEST_HOST" true
make test-live-helper
