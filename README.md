# skillmgr

Deploy [Agent Skills](https://agentskills.io) from git repositories and local
directories, declaratively, from a single `skillmgr.yaml`.

## Description

`skillmgr` is a small CLI that reads a `skillmgr.yaml` describing where your
skills come from — git repositories pinned to a revision, or directories next
to the config file — and makes your skills directories match it: it installs
what is declared, refreshes what moved, and removes what you dropped. The file
is shaped after `pre-commit`'s: a list of repos, each with a revision and the
paths to pull from.

Out of the box it deploys to **both** `.claude/skills/`, which Claude Code
reads, and `.agents/skills/`, the cross-client convention every other Agent
Skills client scans — so one `skillmgr update` serves Claude Code, Cursor,
Codex, Gemini CLI, Copilot, OpenCode and the rest without configuring
anything. A skill folder that also carries a `.claude-plugin/plugin.json` is
copied whole, so plugin-shaped skills keep working too.

What it is not: a registry, a package manager with dependency resolution, or a
skill authoring tool. It moves directories that already exist and validates
them against the specification.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the design and the reasoning behind
it.

## Getting started

### Prerequisites

- `git`, on `PATH` — `skillmgr` shells out to it for every remote source.
- Rust ≥ 1.97.1, only to build from source.

### Installation

```sh
cargo install --git https://github.com/therealm-tech/skillmgr
```

Or download the binary for your platform from the
[releases](https://github.com/therealm-tech/skillmgr/releases).

### Configuration

`skillmgr.yaml`, read from the working directory unless `--config` says
otherwise:

```yaml
repos:
  - repo: https://github.com/toto/tata
    revision: 1.2.3
    paths:
      - path: mydir/
        recurse: true
        exclude: ^toto
  - repo: local
    paths:
      - path: skills
```

| Key | Required | Default | Description |
| --- | --- | --- | --- |
| `targets` | no | `.claude/skills` and `.agents/skills` | Directories the skills are deployed into, each getting a full copy. `~` is expanded; a relative path resolves against the working directory. |
| `repos[].repo` | yes | — | A git URL, a path to a git repository, or `local` for the directory holding the config file. |
| `repos[].revision` | for git | — | Tag, branch or commit to check out. Forbidden on `local`. |
| `repos[].paths[].path` | yes | — | Directory to search, relative to the source root. |
| `repos[].paths[].recurse` | no | `false` | Search the whole subtree instead of the immediate children. |
| `repos[].paths[].exclude` | no | — | Regular expression rejecting skills whose path under `path` matches. |

A `path` that itself holds a `SKILL.md` is taken as one skill. Otherwise its
subdirectories are searched, and a skill found on the way is never descended
into. A `local` source's paths resolve against the config file's directory, so
a config can be shared without its sources moving. See
[examples/skillmgr.yaml](examples/skillmgr.yaml) for a commented config.

Set `targets` yourself to deploy elsewhere — a single directory, or the
user-level pair:

```yaml
targets:
  - ~/.claude/skills
  - ~/.agents/skills
```

A [JSON Schema](schema/skillmgr.schema.json) describes the file. Point your
editor at it for completion and inline errors, by adding this first line to
`skillmgr.yaml`:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/therealm-tech/skillmgr/main/schema/skillmgr.schema.json
```

`skillmgr schema` prints the same document, for a validator that wants it on
stdin. It covers the file's shape; `skillmgr validate` goes further and checks
the sources and the skills themselves.

Every option is also an environment variable:

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `SKILLMGR_CONFIG_FILE` | no | `skillmgr.yaml` | Path to the configuration file. |
| `SKILLMGR_SKILLS_DIR` | no | — | Colon-separated target directories, overriding the config's `targets`. |
| `SKILLMGR_CACHE_DIR` | no | `<cache>/skillmgr` | Where the git checkouts are kept between runs. |
| `SKILLMGR_OFFLINE` | no | `false` | Work from the cache only, contacting no remote. |
| `SKILLMGR_DRY_RUN` | no | `false` | Report what `update` would change, and stop. |
| `SKILLMGR_FORCE` | no | `false` | Let `update` take over a directory it did not install. |
| `SKILLMGR_LOG_FILTER` | no | `info` | `tracing` filter directive, e.g. `skillmgr=debug`. |

### Usage

Deploy everything the config declares:

```sh
skillmgr update
```

See what would change first:

```sh
skillmgr update --dry-run
```

Check the config and every skill it selects, without deploying:

```sh
skillmgr validate
```

Check config files alone, fetching nothing:

```sh
skillmgr validate --config-only skillmgr.yaml
```

List what is currently deployed:

```sh
skillmgr list
```

Print the JSON Schema for the config file:

```sh
skillmgr schema
```

Deploy somewhere else, for this run only:

```sh
skillmgr update --target ~/.claude/skills --target ~/.agents/skills
```

Refresh from the cache on a plane:

```sh
skillmgr update --offline
```

The result goes to stdout and the diagnostics to stderr, so
`skillmgr update > changes.txt` keeps both readable.

### Pre-commit hook

`skillmgr` publishes a hook, so a repository that carries a `skillmgr.yaml`
can keep it honest. In that repository's `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/therealm-tech/skillmgr
    rev: v0.1.0
    hooks:
      - id: skillmgr-validate
```

Pin `rev` to a released tag. The hook runs on every `skillmgr.yaml` a commit
touches and checks that it parses and its rules hold. It fetches nothing, so
it stays fast and works offline — and therefore says nothing about the skills
those sources would yield. Run `skillmgr validate` for that, in CI or by hand.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Apache-2.0 — see [LICENSE](LICENSE).
