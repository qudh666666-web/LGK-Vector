# LGK-Vector 运行说明

操作 Vector DaVinci ECUC 工程前，先读取同目录的 `SKILL.md`。使用同目录的
PowerShell 包装器，不要只单独调用某一个 EXE，也不要直接结束 Host 进程。

目标工程的 Cfg 目录需要 `lgk-vector.json`；若没有，使用
`Initialize-LGKVectorProject.ps1` 创建。修改前先查询，保持改动范围最小，
只生成受影响模块，并在每次会话结束时调用 `shutdown_host`。

本包兼容 Codex、OpenCode、Claude Code 以及任何能读取本文件并运行
PowerShell 的 Agent。包内不包含 Vector 软件、SIP、许可证或客户配置。
