# LGK-Vector 技能包

这是给 Windows 使用者准备的精简运行包，用于调用本机已合法安装的
Vector DaVinci Configurator。它不是源码仓库，包内不包含 DaVinci、SIP、
许可证或任何客户工程。

## 安装

将完整的 `lgk-vector` 文件夹复制到下列任意一个技能目录。不要只复制一个
EXE：CLI、Host 和 PowerShell 脚本必须保持在同一个文件夹内。

| Agent | 工程级安装 | 全局安装 |
| --- | --- | --- |
| Codex | `<工程目录>\.codex\skills\lgk-vector` | `%USERPROFILE%\.codex\skills\lgk-vector` |
| OpenCode | `<工程目录>\.opencode\skills\lgk-vector` | `%USERPROFILE%\.config\opencode\skills\lgk-vector` |
| Claude Code | `<工程目录>\.claude\skills\lgk-vector` | `%USERPROFILE%\.claude\skills\lgk-vector` |

安装后新开一个 Agent 会话，并直接说：`使用 lgk-vector skill 处理当前
DaVinci ECUC 任务。` 不支持原生 Skill 的 Agent 也可以先读取
`lgk-vector\AGENTS.md`，再调用同一套脚本。

## 第一次接入工程

在安装后的 `lgk-vector` 文件夹内执行下面命令，为目标工程创建配置：

```powershell
& ".\Initialize-LGKVectorProject.ps1" `
  -ProjectPath "D:\\Work\\Vehicle\\Cfg" `
  -ToolPath "D:\\VectorSIP"
```

它会在 DaVinci 的 Cfg 目录中创建 `lgk-vector.json`。之后可手工调用
包装器，也可以让安装后的 Agent Skill 自动调用：

```powershell
& ".\Invoke-LGKVector.ps1" `
  -ProjectPath "D:\\Work\\Vehicle\\Cfg" `
  -Request '{"func":"find_module","module":"Com"}'
```

完成工作后，必须通过同一包装器发送 `{ "func": "shutdown_host" }`。

## 验证下载包

接入真实工程前，双击 `test\一键测试EXE.cmd`。看到 `"valid": true` 表示
包内 EXE 配对、中文路径、本地 ECUC 查询、缓存和 Host 正常关闭均已通过。
该测试不会启动 DaVinci，也不能替代目标电脑上的 DaVinci/SIP/许可证验证。

源码、贡献方式和完整维护资料见：
https://github.com/qudh666666-web/LGK-Vector
