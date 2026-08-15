# LGK-Vector runtime instructions

Read sibling `SKILL.md` before operating a Vector DaVinci ECUC project. Use
the sibling PowerShell wrapper instead of calling only one EXE or killing a
Host process.

The target project's Cfg directory needs `lgk-vector.json`; create it with
`Initialize-LGKVectorProject.ps1` when absent. Query before editing, keep every
change scoped, generate only affected modules, and call `shutdown_host` at the
end of every session.

This package is compatible with Codex, OpenCode, Claude Code, and other agents
that can read this file and run PowerShell. It contains no Vector software,
SIP, license, or customer configuration.
