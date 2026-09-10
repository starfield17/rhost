[← Architecture map](../ARCHITECTURE.md)

# Part IX — status and monitor

## 29. `status`

`rhost status <host>` returns one snapshot.

Generic Linux fields:

```text
hostname
OS / kernel / WSL detection
uptime
load
CPU count / utilization
memory
disk
network reachability / RTT
managed sessions
managed jobs
accelerators
```

Accelerators are an extensible list:

```json
{
  "accelerators": [
    {
      "vendor": "nvidia",
      "type": "gpu",
      "name": "NVIDIA GeForce RTX 4060",
      "utilization_percent": 93,
      "memory_used_bytes": 6657199308,
      "memory_total_bytes": 8589934592,
      "temperature_c": 66
    }
  ]
}
```

If `nvidia-smi` is missing or a field is unsupported in WSL, represent it as unavailable. Do not fail the entire host snapshot.

---

## 30. Remote probe design

Avoid one SSH process per metric.

A single `status` snapshot should execute one bounded remote probe that gathers the necessary fields, preferably returning a simple machine-readable intermediate form.

The probe may call:

```text
/proc
uname
df
ps
tmux
nvidia-smi
```

Keep it read-only.

The probe should be versioned so parser expectations are explicit.

Do not install a remote daemon in v0.1 just to collect telemetry.

---

## 31. `watch`

`rhost watch <host>` is a human-facing live monitor.

It repeatedly calls the same application-layer snapshot logic at a configurable interval, usually around 2 seconds.

It must not become a separate state owner.

```text
watch
  │
  ├─ fetch status
  ├─ render
  ├─ sleep
  └─ repeat
```

If network connectivity disappears:

```text
online → reconnecting/offline
```

When connectivity returns, the monitor reconstructs the host state.

It should then rediscover tmux sessions and remote jobs instead of assuming continuity from local memory.

v0.1 can be a terminal UI. A browser dashboard is not required.

---

