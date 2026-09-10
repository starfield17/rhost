[← Architecture map](../ARCHITECTURE.md)

# Part XIV — releases and installation

## 38. Release artifacts

Automate releases with GoReleaser or a small GitHub Actions matrix.

Initial artifacts:

```text
darwin/arm64
darwin/amd64
linux/amd64
linux/arm64
```

Priority:

```text
1. darwin/arm64
2. linux/amd64
3. linux/arm64
4. darwin/amd64
```

The project should be runnable after copying one binary into `$PATH`.

The installer may download the appropriate release.

Do not require Python, Node, or a virtual environment.

---

## 39. Version output

Provide:

```bash
rhost version
```

Include:

```text
version
git commit
build date
Go version
schema version
```

This is important when an Agent reports a remote-control behavior that may depend on CLI version.

---

