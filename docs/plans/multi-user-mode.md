# Multi-User Mode Plan (TODO #13)

**Status**: Planning
**Scope**: Minimal, server-grade multi-user support. No PAM. No GNU shadow-utils. All components permissively licensed.

---

## 1. Where Things Stand Today

The current jonerix baseline is already partially multi-user-capable:

| What exists | Notes |
|-------------|-------|
| `/etc/passwd` | Only `root` and `nobody` |
| `/etc/shadow` | Shadow password file, both entries locked (`!`) |
| `/etc/group` | `root`, `wheel`, `daemon`, `nobody` groups |
| `toybox` | Built with `CONFIG_TOYBOX_SHADOW=y`; includes `login`, `passwd`, `su`, `id`, `groups`, `who`, `whoami` |
| `doas` v6.8.2 | Privilege escalation for `wheel` group members; replaces sudo |
| `openrc` | Init system; `inittab` already references `agetty` on `tty1`–`tty3` |
| `dropbear` | SSH server; supports per-user login but `root` login is disabled |
| `mksh` | `/bin/sh` symlink; `etc/skel/.mkshrc` is already installed by the mksh recipe |

**Critical gap**: `agetty` is referenced in `inittab` but is not shipped. It comes from `util-linux`, which is GPL-2.0+. A permissive replacement is needed.

**Secondary gap**: No `useradd`, `userdel`, `usermod`, `groupadd`, `groupdel` utilities. Toybox does not provide these in 0.8.11.

---

## 2. PAM Analysis — Verdict: Skip It

PAM (Linux-PAM, GPL-2.0) is disqualifying by license. The common alternative `OpenPAM` is BSD-2-Clause and in principle usable, but PAM adds substantial complexity without meaningful benefit for jonerix's server and minimal-desktop use cases:

- musl's `getpwnam`/`getspnam`/`crypt` work correctly with `/etc/passwd` and `/etc/shadow` without PAM.
- toybox `login` reads `/etc/shadow` directly via musl's shadow APIs (enabled by `CONFIG_TOYBOX_SHADOW=y`).
- dropbear has its own authentication path and does not use PAM.
- doas does not use PAM; it reads `/etc/shadow` directly.

**Decision**: No PAM. Authentication is handled natively by toybox `login`, dropbear, and doas — all reading `/etc/shadow` via musl's `getspnam`. This is the same approach used by Alpine Linux and musl-based embedded systems.

---

## 3. Component Inventory

### 3.1 Already Available (no new packages needed)

| Command | Provided by | License | Notes |
|---------|-------------|---------|-------|
| `login` | toybox 0.8.11 | 0BSD | `CONFIG_LOGIN=y` |
| `passwd` | toybox 0.8.11 | 0BSD | `CONFIG_PASSWD=y`; uses `/etc/shadow` |
| `su` | toybox 0.8.11 | 0BSD | `CONFIG_SU=y` |
| `id` | toybox 0.8.11 | 0BSD | `CONFIG_ID=y` |
| `groups` | toybox 0.8.11 | 0BSD | `CONFIG_GROUPS=y` |
| `who` | toybox 0.8.11 | 0BSD | `CONFIG_WHO=y` |
| `whoami` | toybox 0.8.11 | 0BSD | `CONFIG_WHOAMI=y` |
| `doas` | doas 6.8.2 | ISC | Privilege escalation for `wheel` |
| `mksh` skel | mksh R59c | MirOS | `/etc/skel/.mkshrc` already installed |

### 3.2 Missing: `agetty` / Console Login Manager

`agetty` is part of util-linux (GPL-2.0+). Two permissive alternatives exist:

**Option A: `mingetty`**
- License: GPL-2.0 — disqualified.

**Option B: toybox `getty`**
- Toybox has a `getty` applet (`CONFIG_GETTY`). It is currently **not enabled** in `toybox.config`.
- License: 0BSD
- This is the cleanest solution — it ships inside the existing toybox binary with zero new dependencies.
- Limitation: toybox `getty` is simpler than agetty; it lacks auto-baud and some termios flags, but these are irrelevant for server use.

**Option C: `mgetty`**
- License: GPL-2.0+ — disqualified.

**Option D: Write a minimal `jgetty`**
- A permissive getty is ~100 lines of C: open tty, set terminal modes, print hostname, read username, exec `login`.
- MIT-licensed; trivial to maintain.
- Needed only if toybox `getty` proves insufficient (unlikely).

**Recommendation**: Enable toybox `CONFIG_GETTY=y` and update `inittab` to use `/bin/getty` instead of `/bin/agetty`.

### 3.3 Missing: User/Group Management Utilities

Toybox 0.8.11 does **not** include `useradd`, `userdel`, `usermod`, `groupadd`, `groupdel`, `groupmod`, or `chage`. These must come from elsewhere.

**Option A: `shadow-utils`** (the standard `useradd`/etc.)
- License: BSD-3-Clause (shadow 4.x moved away from GPL for userspace tools)
- Actual status: The shadow-utils project (`shadow` on GitHub) is dual-licensed. Core utilities are MIT or BSD; some files retain GPL. The upstream COPYING file lists MIT. **Needs careful audit before including** — some optional components (PAM integration code, libmisc) may be GPL. Build with `--without-pam --without-audit`.
- Verdict: **Potentially usable** with careful build flags. Requires license audit of exact source files compiled.

**Option B: `busybox-nologin` / Alpine's `shadow` package approach**
- Alpine ships a custom `shadow` package derived from the upstream shadow-utils with PAM stripped. License status same as Option A.
- Not directly usable as a jpkg recipe without the same audit.

**Option C: Write `juseradd` — a minimal user management tool**
- A self-contained `juseradd`/`juserdel`/`jgroupadd` in ~500 lines of C.
- License: MIT (new jonerix-native code).
- Reads/writes `/etc/passwd`, `/etc/shadow`, `/etc/group`, `/etc/gshadow` atomically using `lckpwdf`/`ulckpwdf` (POSIX).
- musl provides `lckpwdf`, `putpwent`, `putspent`, `putgrent`.
- **Recommended long-term path** since it avoids any license ambiguity and keeps the binary small.

**Option D: Shell-script user management**
- `/bin/adduser` and `/bin/deluser` as mksh scripts (~100 lines each).
- Alpine uses this approach; their `adduser` script is MIT-licensed.
- Pros: immediately usable, zero new compiled code.
- Cons: shell scripts running as root with direct `/etc/shadow` manipulation need to handle concurrent access carefully (requires `lckpwdf` equivalent — not scriptable). Acceptable if single-user-at-a-time administration is assumed.
- **Recommended for v1** — deliver working adduser/deluser scripts; replace with `juseradd` binary in v2.

**Recommendation**:
- v1: MIT-licensed shell scripts (`adduser`, `deluser`, `addgroup`, `delgroup`) modeled on Alpine's approach.
- v2: Replace with a `juseradd` C binary if concurrency or correctness issues arise.

### 3.4 Password Hashing

musl provides `crypt()` with SHA-512 (`$6$`) support. toybox `passwd` uses this via musl. No additional library needed.

To verify: the toybox `passwd` command invokes `crypt()` which in musl supports:
- `$1$` — MD5 (weak, avoid)
- `$5$` — SHA-256
- `$6$` — SHA-512 (default, recommended)

Shadow entries should use `$6$` hashes. The adduser scripts and any password-setting code must use `openssl passwd -6` or toybox `passwd` (which calls musl `crypt` with SHA-512 automatically).

---

## 4. `/etc/passwd`, `/etc/shadow`, `/etc/group` Schema

### 4.1 Expanded Default `/etc/passwd`

The default config should include standard system accounts:

```
root:x:0:0:root:/root:/bin/sh
bin:x:1:1:bin:/bin:/bin/false
daemon:x:2:2:daemon:/sbin:/bin/false
adm:x:3:4:adm:/var/adm:/bin/false
lp:x:4:7:lp:/var/spool/lpd:/bin/false
sync:x:5:0:sync:/sbin:/bin/sync
shutdown:x:6:0:shutdown:/sbin:/bin/false
halt:x:7:0:halt:/sbin:/bin/false
mail:x:8:12:mail:/var/spool/mail:/bin/false
news:x:9:13:news:/var/spool/news:/bin/false
uucp:x:10:14:uucp:/var/spool/uucp:/bin/false
operator:x:11:0:operator:/root:/bin/false
sshd:x:22:22:sshd:/var/empty/sshd:/bin/false
nobody:x:65534:65534:nobody:/nonexistent:/bin/false
```

System user UIDs: 1–499 (matches `CONFIG_TOYBOX_UID_SYS=100`, `CONFIG_TOYBOX_UID_USR=500`).
Interactive user UIDs: 500+.

### 4.2 Expanded Default `/etc/shadow`

All system accounts locked (`!` or `*`):

```
root:!:19808:0:99999:7:::
bin:!:19808:::::::
daemon:!:19808::::::::
...
nobody:!:19808::::::::
```

Root password is locked by default; must be set on first boot or by the installer.

### 4.3 Expanded Default `/etc/group`

```
root:x:0:root
bin:x:1:root,bin,daemon
daemon:x:2:root,bin,daemon
adm:x:4:root,adm,daemon
tty:x:5:
disk:x:6:root,adm
lp:x:7:lp
mem:x:8:
kmem:x:9:
wheel:x:10:
mail:x:12:mail
news:x:13:news
uucp:x:14:uucp
audio:x:29:
video:x:28:
cdrom:x:19:
floppy:x:11:
sshd:x:22:
users:x:100:
nobody:x:65534:
```

`users` group (GID 100) is the default group for new interactive users.

### 4.4 `/etc/gshadow`

musl supports `/etc/gshadow` for group passwords (rarely used). Add an empty `gshadow` file for compatibility:

```
root:::root
wheel:::
users:::
...
```

---

## 5. Home Directory Management

### 5.1 `/etc/skel`

The mksh recipe already installs `/etc/skel/.mkshrc`. Expand skel to include:

```
/etc/skel/
├── .mkshrc          (already provided by mksh recipe)
├── .profile         (minimal POSIX profile: PATH, umask, editor)
└── .config/         (empty dir for XDG compliance)
```

No `.bashrc` (bash not in runtime). No `.zshrc` (zsh is develop-image only).

### 5.2 Home Directory Creation

`adduser` script creates `/home/<username>` with `chmod 750` and copies skel:

```sh
mkdir -p /home/$username
cp -a /etc/skel/. /home/$username/
chown -R $uid:$gid /home/$username
chmod 750 /home/$username
```

### 5.3 Home Directory Permissions

| Path | Owner | Mode | Rationale |
|------|-------|------|-----------|
| `/home/` | root:root | 755 | Directory itself readable |
| `/home/<user>/` | user:users | 750 | Private; group-readable for shared systems |
| `/root/` | root:root | 700 | Already set in Dockerfiles |

---

## 6. File Permission Defaults

### 6.1 SUID Binaries

Multi-user mode requires SUID on a small set of binaries:

| Binary | Required SUID? | Notes |
|--------|----------------|-------|
| `/bin/su` | Yes | Must setuid root to switch users |
| `/bin/passwd` | Yes | Must write `/etc/shadow` as root |
| `/bin/doas` | Yes | Already SUID — configured by OpenDoas install |
| `/bin/login` | Yes | Needs to setuid for PAM-less login |
| `/bin/newgrp` | Yes (if provided) | Not in toybox; skip for v1 |

toybox is built with `CONFIG_TOYBOX_SUID=y`, which means toybox will handle SUID dropbear correctly for `su`, `passwd`, and `login`.

These must be set in the Dockerfile or image assembly:

```sh
chmod u+s /bin/su /bin/passwd /bin/login
```

### 6.2 Device Permissions for TTYs

`/dev/tty*` must be owned by `tty` group (GID 5) and group-writable for `write`/`wall` to work:

```sh
# set during devtmpfs setup in OpenRC sysinit
chown root:tty /dev/tty* && chmod 620 /dev/tty*
```

OpenRC's `devfs` service typically handles this via udev rules or mdev. For jonerix, a small `rc.devperms` init script is the appropriate mechanism (see Section 8).

### 6.3 Default umask

`/etc/profile` already sets `umask 022`. For a more private multi-user default, consider `umask 027` (new files not readable by others). Keep `022` for v1 to avoid breaking build workflows.

---

## 7. Integration with OpenRC

### 7.1 `inittab` Change: `agetty` → `getty`

**Current** (broken — `agetty` not installed):
```
tty1::respawn:/bin/agetty 38400 tty1 linux
```

**Proposed** (using toybox `getty`):
```
tty1::respawn:/bin/getty 38400 tty1
tty2::respawn:/bin/getty 38400 tty2
tty3::respawn:/bin/getty 38400 tty3
```

toybox `getty` accepts `<speed> <tty>` and execs `login` after reading the username. No termtype argument needed.

### 7.2 New OpenRC Services

Add these init scripts for multi-user support:

**`/etc/init.d/hostname`** (may already exist via OpenRC)
- Sets hostname from `/etc/hostname` at boot.
- OpenRC 0.54 includes this service.

**`/etc/init.d/urandom`**
- Seeds `/dev/urandom` from `/var/lib/urandom/seed` at boot.
- Needed for password hashing randomness on first boot.
- OpenRC 0.54 includes this service.

**`/etc/init.d/devperms`** (new, ~20 lines)
- Fixes TTY permissions after devtmpfs mount.
- Runs in `sysinit` runlevel.
- Sets `tty` group ownership on `/dev/tty*`.

### 7.3 `/etc/securetty`

`login` optionally reads `/etc/securetty` to restrict root logins to listed TTYs. For security, create `/etc/securetty` with only:

```
tty1
```

This allows root local console login only from `tty1`. SSH root login is already blocked via dropbear's `-w` flag.

### 7.4 Runlevels

No changes to runlevel structure needed. The `getty` processes run in the default runlevel via `inittab` `respawn` entries, not as OpenRC services.

---

## 8. Dropbear SSH Multi-User Integration

Dropbear already supports multi-user SSH login. Required configuration:

- `/etc/dropbear/` — host keys (already set up)
- `/var/empty/sshd/` — chroot dir for privilege separation (already in Dockerfile)
- `sshd` privilege separation: dropbear uses its own mechanism, not PAM
- Per-user `~/.ssh/authorized_keys` — standard; no changes needed

In the OpenRC `dropbear` init script, ensure `-R` (generate host keys if missing) is set for first-boot scenarios.

---

## 9. Packages to Add or Modify

### 9.1 toybox.config Changes (enable `getty`)

In `packages/core/toybox/toybox.config`, add to the User management section:

```
CONFIG_GETTY=y
CONFIG_USERADD=y    # if/when toybox adds it — not in 0.8.11
CONFIG_USERDEL=y    # if/when toybox adds it
CONFIG_GROUPADD=y   # if/when toybox adds it
```

For now, only `CONFIG_GETTY=y` is relevant — toybox 0.8.11 has `getty` but not the user management commands.

### 9.2 New Package: `shadow` or `juseradd` (v1: shell scripts)

For v1, deliver as a `jonerix-user-tools` package containing shell scripts:

- `/bin/adduser` — creates user (home dir, passwd entry, shadow entry)
- `/bin/deluser` — removes user (optionally removes home dir)
- `/bin/addgroup` — creates group
- `/bin/delgroup` — removes group
- `/bin/lslogins` — lists users (optional; toybox `who` covers some of this)

These scripts use toybox primitives (`grep`, `awk`, `sed`, `cut`, `install`, `chown`, `chmod`) available in the existing toybox build. They write to `/etc/passwd`, `/etc/shadow`, `/etc/group` using atomic replace (write to `.new`, then `mv`).

License: MIT (new jonerix code).

**v2 path**: Replace scripts with a compiled `juseradd` C binary or audit `shadow-utils` for a clean BSD-licensed subset.

### 9.3 Config File Updates

| File | Change |
|------|--------|
| `config/defaults/etc/passwd` | Expand with standard system accounts (see §4.1) |
| `config/defaults/etc/shadow` | Add locked entries for all new system accounts |
| `config/defaults/etc/group` | Expand groups (see §4.3) |
| `config/defaults/etc/gshadow` | Create new file (empty group passwords) |
| `config/defaults/etc/securetty` | Create new file (tty1 only for root) |
| `config/defaults/etc/shells` | Add `/bin/mksh` alongside `/bin/sh` |
| `config/openrc/inittab` | Change `agetty` to `getty` |

### 9.4 Dockerfile Changes

In `Dockerfile.minimal` and `Dockerfile`, the rootfs assembly should:

1. Install `mksh` in the package list (for `/bin/mksh` and `/etc/skel`)
2. Set SUID bits on `su`, `passwd`, `login` after package install
3. Copy the new config files (`gshadow`, `securetty`)
4. Create `/var/empty/sshd` with correct permissions (`chmod 700, chown root:root`)
5. Create `/etc/skel` from mksh recipe output (already done)

---

## 10. First-Boot Root Password Setup

Root's password is locked by default (`!` in shadow). A first-boot mechanism is needed:

**Option A: Cloud-init-lite**
The `cloud-init-lite` shell script described in DESIGN.md can handle setting the root password from the cloud metadata service or a cloud-config user-data file.

**Option B: Serial console setup**
For bare-metal: on first boot, if no password is set and a console is attached, prompt to set a root password before starting getty. A small `firstboot` OpenRC service handles this.

**Option C: Build-time password injection**
For container images: set password via `RUN echo 'root:password' | chpasswd` equivalent using `openssl passwd -6` during Dockerfile assembly.

**Recommendation**: For container images (OCI), use Option C. For bare-metal/cloud, implement a `firstboot` service (Option B/A).

---

## 11. musl libc Capabilities and Limitations

musl provides all APIs needed for multi-user operation without PAM:

| API | musl support | Used by |
|-----|-------------|---------|
| `getpwnam`, `getpwuid`, `getpwent` | Full | login, doas, dropbear |
| `getspnam`, `getspent` | Full (with `_GNU_SOURCE`) | login, passwd, doas |
| `getgrnam`, `getgrgid`, `getgrent` | Full | id, groups, doas |
| `crypt` (SHA-512 `$6$`) | Full | passwd |
| `setuid`, `setgid`, `initgroups` | Full | su, login |
| `lckpwdf`, `ulckpwdf` | Full | user management tools |
| `login_tty`, `openpty` | Not in musl | Not needed for server |
| NSS (`nsswitch.conf`) | Not supported | Not needed — files only |

musl intentionally does not implement NSS/`nsswitch.conf`. All user lookups go directly to `/etc/passwd` and `/etc/shadow`. This is a feature: it eliminates the LDAP/SSSD/Kerberos complexity that PAM normally enables. jonerix is not designed for enterprise directory integration.

---

## 12. Security Considerations

### 12.1 Shadow File Permissions

`/etc/shadow` must be `root:root 000` (unreadable by anyone but root via open). musl's `getspnam` uses a direct open by root-privileged processes.

### 12.2 SUID Surface

Minimizing SUID binaries reduces privilege escalation risk:

| Binary | SUID needed | Alternative |
|--------|-------------|-------------|
| `su` | Yes | — |
| `passwd` | Yes | — |
| `login` | Yes | — |
| `doas` | Yes | — |
| `ping` | No | Use CAP_NET_RAW or `doas ping` |
| `mount` | No | Root-only system |

Set SUID only on the four binaries listed above. Do not set SUID on `ping` — use `doas` if non-root ping is needed.

### 12.3 Account Lockout

toybox `login` does not implement account lockout (failed attempt tracking). For a server with SSH as the primary remote access path, this is acceptable because:

- dropbear handles SSH brute-force protection separately
- Console logins are physical-access scenarios

For v2, consider adding a `faillock`-equivalent or recommend placing console TTYs behind dropbear's rate limiting for remote serial consoles.

---

## 13. Implementation Sequence

| Step | What | Effort | Priority |
|------|------|--------|----------|
| 1 | Enable `CONFIG_GETTY=y` in toybox.config; rebuild toybox package | Low | P0 |
| 2 | Update `config/openrc/inittab` to use `/bin/getty` | Trivial | P0 |
| 3 | Expand `config/defaults/etc/passwd`, `shadow`, `group` | Low | P0 |
| 4 | Add `config/defaults/etc/gshadow` and `securetty` | Trivial | P0 |
| 5 | Update `config/defaults/etc/shells` to include `/bin/mksh` | Trivial | P0 |
| 6 | Set SUID bits in Dockerfiles (`chmod u+s /bin/su /bin/passwd /bin/login`) | Trivial | P0 |
| 7 | Write `adduser`/`deluser`/`addgroup`/`delgroup` shell scripts | Medium | P1 |
| 8 | Create a `jonerix-user-tools` jpkg recipe for the scripts | Low | P1 |
| 9 | Add `devperms` OpenRC init script for TTY permissions | Low | P1 |
| 10 | Implement `firstboot` service for root password setup | Medium | P2 |
| 11 | Implement `cloud-init-lite` for cloud deployments | High | P2 |

P0 = required for basic multi-user console login to work
P1 = required for usable administration
P2 = required for production deployments

---

## 14. What Multi-User Mode Does NOT Include (v1 Scope)

The following are explicitly out of scope for v1:

- **LDAP/NIS/directory integration**: musl has no NSS; files-only authentication is the design.
- **Kerberos/GSSAPI**: No use case in jonerix's target deployment model.
- **PAM**: Rejected (GPL or complexity without benefit).
- **`newgrp`**: Not in toybox; rarely needed on servers.
- **`chfn`, `chsh`**: toybox does not provide these; users can edit `/etc/passwd` directly as root.
- **`finger`, `w`**: toybox `who` covers basic session listing; `finger` is not needed.
- **Quota management**: No `quotatools` equivalent; out of scope.
- **ACLs**: No `getfacl`/`setfacl`; standard Unix permissions are sufficient.

---

## 15. License Summary of New Components

| Component | License | Source |
|-----------|---------|--------|
| toybox `getty` applet | 0BSD | toybox (already in codebase) |
| `adduser`/`deluser` scripts | MIT | New jonerix code |
| `addgroup`/`delgroup` scripts | MIT | New jonerix code |
| `devperms` init script | MIT | New jonerix code |
| `firstboot` init script | MIT | New jonerix code |
| Config file additions | MIT | New jonerix data |

No GPL components introduced. All new code is MIT-licensed jonerix-native work.
