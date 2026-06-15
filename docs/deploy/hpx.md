# HPX Deployment

HPX is the permanent bkgrnd host. WOPR is legacy and should not be treated as the active deployment target.

## Live Runtime

- Public app: `https://bkgrnd.bedl.am/`
- SSH host alias: `hpx`
- Dokploy/Compose root: `/etc/dokploy/compose/wopr-apps/code`
- App repository directory: `/etc/dokploy/compose/wopr-apps/code/bkgrnd`
- Runtime container: `bkgrnd-hpx`
- Data volume: `/etc/dokploy/compose/wopr-apps/data/bkgrnd:/data`

## GitHub Source

The HPX app directory should be a checkout of:

```text
https://github.com/bedlamlabs/bkgrnd.git
```

The expected branch is `main`.

## Bootstrap Or Repair The HPX Checkout

Use the reviewed wrapper from the repo root:

```bash
scripts/hpx/connect-github.sh connect
```

This backs up any existing non-git HPX app directory to a timestamped sibling, clones the GitHub repo, and preserves common local env files when present. It does not rebuild or restart the container.

To rebuild after the checkout is connected:

```bash
scripts/hpx/connect-github.sh deploy
```

Do not run destructive Docker cleanup as part of this flow. The data volume is intentionally left untouched.
