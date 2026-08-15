# LGK-Vector: cross-agent instructions

This repository is an open-source, local bridge for a user's licensed Vector
DaVinci Configurator installation. It contains no DaVinci, SIP, license, or
customer ECUC material. The same workflow is intended for Codex, OpenCode,
Claude-compatible agents, and agents that can read project instructions and
run PowerShell.

## Start here

1. Read this file before changing a Vector ECUC project.
2. For the full request contract and examples, read `SKILL.md` and then the
   relevant section of `docs/跨工程接入.md`.
3. Use `scripts/Invoke-LGKVector.ps1` as the normal entry point. It starts,
   checks, and closes the paired resident Host correctly.
4. Each target DaVinci Cfg directory owns only its `lgk-vector.json`; keep the
   tool source in one shared directory, not copied into customer projects.

## Safe ECUC workflow

- Inspect first: use `find_module`, `inspect_ecuc_containers`,
  `find_module_template`, and `get_param_definition` before proposing a change.
- For DaVinci configuration, use LGK-Vector requests rather than manually
  editing ARXML. Use `edit_file` only for a scoped, reviewed text change with
  exact `expected` source text.
- Before `edit_file`, `import_dbc`, or `update_project`, save and close the
  same project in the DaVinci GUI. Do not make a disk edit while its unsaved GUI
  model can overwrite it.
- Send mutations (`edit_file`, `import_dbc`, `update_project`,
  `auto_solve_errors`, `generate_code`) one at a time. Validate errors and
  generate only the affected module unless an explicit all-module generation
  is intended.
- Always finish a session with `{ "func": "shutdown_host" }`, including after
  a failed request. Never kill the Host as the normal cleanup path.
- Never put a customer DPA, ARXML, DBC, generated code, license, token, log,
  or Vector binary in this repository or its release package.

## Common calls

```powershell
# Create the project-local configuration and run static preflight.
& ".\scripts\Initialize-LGKVectorProject.ps1" `
  -ProjectPath "D:\\Work\\Vehicle\\Cfg" -ToolPath "D:\\VectorSIP"

# Normal read-only query. The wrapper starts the matching local Host if needed.
& ".\scripts\Invoke-LGKVector.ps1" `
  -ProjectPath "D:\\Work\\Vehicle\\Cfg" `
  -Request '{"func":"find_module","module":"Com"}'

# Required normal cleanup.
& ".\scripts\Invoke-LGKVector.ps1" `
  -ProjectPath "D:\\Work\\Vehicle\\Cfg" `
  -Request '{"func":"shutdown_host"}'
```

Run requests from the selected tool installation, or provide an explicit
`-ExecutablePath` for a release package. `lgk-vector.json` contains the local
`tool_path`; its directory determines the project path.

## Agent compatibility

- **Codex:** load the root `SKILL.md` after installing it with
  `scripts/Install-LGKVectorSkill.ps1`.
- **OpenCode:** it reads this root `AGENTS.md`; it can also discover the native
  `.agents/skills/lgk-vector/SKILL.md` entry in a cloned repository.
- **Other agents:** treat this file as the authoritative project workflow and
  invoke the same wrapper. No agent-specific API is required.

If an agent cannot read project rule files automatically, tell it explicitly:
“Read `AGENTS.md` and `SKILL.md` before operating LGK-Vector.”

## Testing and releases

The source repository intentionally has one top-level `tests/` directory:

- `tests/local_ops.rs`, `tests/onboarding/`, and `tests/open-source/` are for
  maintainers and CI.
- `tests/release/` is the small synthetic EXE self-test source. The package
  script maps it to the single user-facing `test/` directory in the minimal
  Agent Skill ZIP; the runtime itself is mapped to `lgk-vector/`.

For a downloaded release, double-click `test/一键测试EXE.cmd`. It verifies the
two EXEs and local synthetic behavior; it does not prove a proprietary
DaVinci/SIP generation environment is valid. Contributors run the commands in
`tests/README.md` before publishing and update `CHANGELOG.md` for every public
behavior, interface, wrapper, or Skill change.
