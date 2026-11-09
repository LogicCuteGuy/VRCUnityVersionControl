# LogicCuteGuy VCS (`com.logiccuteguy.vcs`)

Self-hosted Plastic-SCM-style Version Control for VRChat Unity projects.

## Install via Unity Package Manager (Git URL)

In Unity Editor:
1. Open **Window > Package Manager**
2. Click **`+` > Add package from git URL...**
3. Enter:
   ```text
   https://github.com/LogicCuteGuy/VRCUnityVersionControl.git?path=/Packages/com.logiccuteguy.vcs
   ```

## Quick Start for Team Members

1. Connect to the team Tailscale network.
2. Open **Window > LogicCuteGuy VCS** in Unity.
3. Go to the **Workspace** tab:
   - Server URL: `http://100.65.127.80:8087`
   - Repository: `vrc-world`
   - Enter your username and password and click **Login & Connect Workspace**.
4. Click **Update Workspace** to pull the latest changeset, or **Checkin Selected** to submit your changes!
