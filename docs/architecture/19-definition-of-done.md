[← Architecture map](../ARCHITECTURE.md)

# Part XIX — Definition of done

## 53. First release

The first release is usable when a coding agent can execute this workflow through only the shipped Skill and binary:

```text
discover/doctor host
       ↓
sync code
       ↓
run stateless test
       ↓
inspect result
       ↓
open persistent interactive session when needed
       ↓
start durable long job
       ↓
continue other work
       ↓
poll logs/status after reconnect
```

And a human can independently run:

```text
rhost watch <host>
rhost session attach <host> <session>
```

without changing the runtime model.

The remote host must feel like a reusable compute node, not like a collection of handcrafted SSH commands.

---

