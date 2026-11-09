# ChibiVCS (`com.logiccuteguy.vcs`)

Self-hosted, Plastic-SCM-style Version Control System built specifically for **VRChat Unity teams** — 100% open source (MIT).

- **Server (`Server/`)**: Rust (`axum` + `rusqlite` WAL + SHA-256 content-addressed blob store + SSE live events)
- **CLI (`Server/crates/chibivcs-cli`)**: `chibi` command-line client for scripting, CI, and admin operations
- **Unity Package (`Packages/com.logiccuteguy.vcs`)**: Native Unity Editor integration (`LogicCuteGuy.Vcs.Editor`) with Pending Changes, Incoming Changesets, Branches, Exclusive Scene/Prefab Locks, Auto-Checkout Save Guard, and Project Window status overlays.

---

## Install in Unity via Git URL

Open **Window > Package Manager** → **`+` > Add package from git URL...**:

```text
https://github.com/LogicCuteGuy/VRCUnityVersionControl.git?path=/Packages/com.logiccuteguy.vcs
```

---

## Server Commands

```bash
# Create or update users
chibivcs-server --data-dir /var/lib/chibivcs user add LogicCuteGuy --password "<pass>" --admin
chibivcs-server --data-dir /var/lib/chibivcs user add kamikenza --password "<pass>"

# Create a repository
chibivcs-server --data-dir /var/lib/chibivcs repo create vrc-world --description "VRChat World Project"

# Start server on Tailscale IP:8087
chibivcs-server --data-dir /var/lib/chibivcs serve --bind 100.65.127.80:8087
```

## License

MIT
