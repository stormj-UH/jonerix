# Signing keys for jonerix packages

## Trust model

jonerix uses Ed25519 detached signatures for package integrity.

- Single org-level signing key, currently **jonerix-2026**.
- The corresponding public key (32 bytes raw binary) ships at
  `/etc/jpkg/keys/jonerix-2026.pub` on every jonerix host via the
  `jonerix-keys` package (or equivalent).
- The secret key (64-byte raw binary: seed[32] || pubkey[32]) lives
  **only** in the GitHub Actions secret `JPKG_SIGNING_KEY`.
- Signature files are written as `<package>.jpkg.sig` (64 bytes raw
  binary, Ed25519 R‖S).

> Note on naming: the secret is called `JPKG_SIGNING_KEY` for
> historical reasons but the value is **base64-encoded raw binary**,
> not a PEM-armoured key. `jpkg`'s `read_secret_key` function accepts
> only the 64-byte raw format.

---

## Phase 0 rollout (current state)

- Signatures are **optional**. `jpkg install` logs a WARN if a `.sig`
  file is absent; it does not refuse to install the package.
- Once `JPKG_SIGNING_KEY` is set in the repository secrets, every
  `jpkg build` call in CI will pass `--sign-key` and produce a signed
  `.jpkg`.
- Existing unsigned packages can be retroactively signed with
  `jpkg resign` once Worker E lands that subcommand.
- Phase 2 (future) will flip `signature_policy` to `require` on
  production hosts, at which point unsigned packages will be rejected.

---

## How the key flows through CI

```
GitHub secret JPKG_SIGNING_KEY (base64 raw 64-byte key)
  │
  │  "Materialise signing key" step in publish-packages.yml
  │  (runs on the Ubuntu host runner, before docker)
  ▼
/tmp/jpkg-signing-XXXXXX.sec  (mode 0600, raw 64-byte binary)
  │  exported as JPKG_SIGN_KEY into GITHUB_ENV
  ▼
docker run ... -v "$JPKG_SIGN_KEY:$JPKG_SIGN_KEY:ro" \
               -e "JPKG_SIGN_KEY=$JPKG_SIGN_KEY"
  │
  │  ci-build-{aarch64,x86_64}.sh
  ▼
jpkg build <recipe_dir> --build-jpkg --output /var/cache/jpkg \
    --sign-key "$JPKG_SIGN_KEY"
  │
  ▼
<package>-<version>-<arch>.jpkg       (package archive)
<package>-<version>-<arch>.jpkg.sig   (detached Ed25519 signature)
  │
  │  "Remove signing key" step (if: always())
  ▼
rm -f "$JPKG_SIGN_KEY"                (key wiped from runner disk)
```

When the secret is absent the materialise step emits a GitHub Actions
warning and skips the file-write. The build step runs without
`JPKG_SIGN_KEY` set, and the scripts fall back to unsigned builds.

---

## Setting the secret on a fresh fork

1. Generate a key (see next section) and extract the base64-encoded
   secret key string.
2. Set the secret in the repository (requires `admin` access):

   ```sh
   # From a file containing the raw 64-byte binary secret key:
   gh secret set JPKG_SIGNING_KEY \
     --repo <owner>/<repo> \
     --body "$(base64 < /path/to/jonerix-2026.sec)"
   ```

   Or interactively (gh will prompt for the value):

   ```sh
   gh secret set JPKG_SIGNING_KEY --repo <owner>/<repo>
   ```

3. Verify the secret appears (value is redacted):

   ```sh
   gh secret list --repo <owner>/<repo>
   ```

4. Trigger a manual `publish-packages` run. The "Materialise signing
   key" step should log `jpkg signing key materialised.` instead of
   the unsigned warning.

---

## Generating a fresh signing key

```sh
# Generate keypair — writes jonerix-2026.pub (32 B) and
# jonerix-2026.sec (64 B) to the current directory.
jpkg keygen --out jonerix-2026

# Confirm sizes:
ls -l jonerix-2026.pub jonerix-2026.sec
# jonerix-2026.pub  32 bytes  (Ed25519 public key, raw binary)
# jonerix-2026.sec  64 bytes  (seed || pubkey, raw binary, mode 0600)

# Encode the secret key for GitHub:
base64 < jonerix-2026.sec

# The public key ships in the OS image. Place it in the right
# location so jpkg can find it during verification:
#   packages/core/jonerix-keys/files/etc/jpkg/keys/jonerix-2026.pub
# Bump the jonerix-keys recipe version and rebuild.

# Destroy the plaintext secret key once it is stored in GitHub:
rm -P jonerix-2026.sec   # or shred(1) if available
```

---

## Key rotation procedure

1. Generate the new key, e.g. `jonerix-2027`:

   ```sh
   jpkg keygen --out jonerix-2027
   ```

2. Add `jonerix-2027.pub` to `packages/core/jonerix-keys/` alongside
   the existing `jonerix-2026.pub`.  Both keys are trusted during the
   overlap window — jpkg verifies against all keys in
   `/etc/jpkg/keys/`.

3. Update `JPKG_SIGNING_KEY` in GitHub to the new secret key:

   ```sh
   gh secret set JPKG_SIGNING_KEY \
     --repo <owner>/<repo> \
     --body "$(base64 < jonerix-2027.sec)"
   ```

4. Ship a new `jonerix-keys` jpkg containing both `.pub` files.
   Deploy it to all hosts before retiring the old key.

5. After all hosts have the updated `jonerix-keys` package, remove
   `jonerix-2026.pub` from the recipe and bump the version again.

6. Remove the old public key from the repository tree and delete
   plaintext copies of the old secret key.

---

## signature_policy values

`signature_policy` controls how `jpkg install` reacts to missing or
invalid signatures. It is read from `/etc/jpkg/jpkg.conf` (field
`signature_policy`).

| Value     | Behaviour |
|-----------|-----------|
| `ignore`  | Signatures are not checked at all. Fastest; no security. |
| `warn`    | Missing or unverifiable signatures emit a WARN log but installation proceeds. **Current default (Phase 0).** |
| `require` | Installation is aborted if a valid signature cannot be found. Use this on production hosts once all packages are signed. |

To check the current policy on a running system:

```sh
grep signature_policy /etc/jpkg/jpkg.conf
```

To switch a host to `require` mode:

```sh
# Edit /etc/jpkg/jpkg.conf and set:
signature_policy = "require"
```

Do **not** flip to `require` until every package in your INDEX has a
corresponding `.sig` file and the public key is deployed on the host.
