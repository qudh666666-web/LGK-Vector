# LGK-Vector skill package

This is the small Windows runtime package for a locally licensed Vector
DaVinci Configurator installation. It is not the source repository and does
not contain DaVinci, SIP content, licenses, or customer projects.

## Install

Copy the complete `lgk-vector` folder into one of the following skill folders.
Do not copy only one EXE: the CLI, Host, and PowerShell scripts must remain
together.

| Agent | Project installation | Global installation |
| --- | --- | --- |
| Codex | `<project>\.codex\skills\lgk-vector` | `%USERPROFILE%\.codex\skills\lgk-vector` |
| OpenCode | `<project>\.opencode\skills\lgk-vector` | `%USERPROFILE%\.config\opencode\skills\lgk-vector` |
| Claude Code | `<project>\.claude\skills\lgk-vector` | `%USERPROFILE%\.claude\skills\lgk-vector` |

Open a new agent session afterwards and say: `Use the lgk-vector skill for this
DaVinci ECUC task.` Agents that do not implement native skills can read
`lgk-vector\AGENTS.md` and use the same scripts.

## First project

From the installed `lgk-vector` folder, create the project-local configuration:

```powershell
& ".\Initialize-LGKVectorProject.ps1" `
  -ProjectPath "D:\\Work\\Vehicle\\Cfg" `
  -ToolPath "D:\\VectorSIP"
```

This creates `lgk-vector.json` in the DaVinci Cfg directory. Then call the
wrapper, or let the installed Agent Skill call it:

```powershell
& ".\Invoke-LGKVector.ps1" `
  -ProjectPath "D:\\Work\\Vehicle\\Cfg" `
  -Request '{"func":"find_module","module":"Com"}'
```

After work is complete, always send `{ "func": "shutdown_host" }` through the
same wrapper.

## Test the download

Before connecting a real project, double-click `test\一键测试EXE.cmd`. A result
with `"valid": true` proves the packaged EXE pair, Unicode paths, local ECUC
inspection, cache behavior, and normal Host shutdown. It does not start or
validate proprietary DaVinci generation.

For source code, contribution, and detailed maintenance documentation, visit
https://github.com/qudh666666-web/LGK-Vector.
