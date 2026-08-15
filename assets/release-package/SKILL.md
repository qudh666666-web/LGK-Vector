---
name: lgk-vector
description: Use this skill for fast, verified Vector DaVinci ECUC inspection, edits, generation, DBC import, and normal resident-host shutdown.
---

# LGK-Vector

Use the sibling `Invoke-LGKVector.ps1` wrapper for all requests. This runtime
requires a lawful local DaVinci/SIP installation and a project-local
`lgk-vector.json` in the target Cfg directory. If configuration is absent, use
the sibling `Initialize-LGKVectorProject.ps1` first.

## Fast default workflow

1. Query only what is needed: `find_module`, `get_param_definition`,
   `locate_container`, or `inspect_ecuc_containers`.
2. For an ECUC change, save and close the same DaVinci GUI project, read the
   exact target text, and send one scoped `edit_file` request with `expected`.
3. Run `generate_code` for the affected module only. If it fails, obtain
   `get_errors_list` for that module; do not retry blindly or generate all.
4. Report ECUC changes separately from generated C/H/LSL output.
5. End every session with `{ "func": "shutdown_host" }` through the wrapper.

## Guardrails

- Never manually rewrite ECUC ARXML when a verified LGK-Vector request can
  make the change.
- Send `edit_file`, `import_dbc`, `update_project`, `auto_solve_errors`,
  `generate_code`, and `shutdown_host` as separate requests.
- `auto_solve_errors` requires a fresh error list, explicit user approval, and
  `confirmed:true`.
- Before `import_dbc` or `update_project`, save and close the DaVinci GUI.
- Use only the supported functions: `inspect_ecuc_containers`, `find_module`,
  `find_module_template`, `get_param_definition`, `locate_container`,
  `edit_file`, `get_errors_list`, `auto_solve_errors`, `generate_code`,
  `update_project`, `import_dbc`, and `shutdown_host`.

## Example

```powershell
& "<skill-root>\Invoke-LGKVector.ps1" `
  -ProjectPath "D:\\Work\\Vehicle\\Cfg" `
  -Request '{"func":"inspect_ecuc_containers","module":"Com","container":"ComSignal"}'
```

Read sibling `AGENTS.md` for cross-agent setup and the exact project workflow.
