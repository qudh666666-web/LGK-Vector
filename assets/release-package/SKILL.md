---
name: lgk-vector
description: 用于快速、可验证地完成 Vector DaVinci ECUC 查询、修改、生成、DBC 导入和常驻 Host 正常关闭。
---

# LGK-Vector

所有请求都使用同目录的 `Invoke-LGKVector.ps1` 包装器。本运行包要求电脑中
已合法安装匹配的 DaVinci/SIP，并且目标工程 Cfg 目录内存在
`lgk-vector.json`；若配置不存在，先使用同目录的
`Initialize-LGKVectorProject.ps1`。

## 默认快速流程

1. 只查询必要内容：`find_module`、`get_param_definition`、
   `locate_container` 或 `inspect_ecuc_containers`。
2. 修改 ECUC 前，保存并关闭同一个 DaVinci GUI 工程，读取准确原文后发送
   一条带 `expected` 的小范围 `edit_file` 请求。
3. 只对受影响模块调用 `generate_code`。若失败，再读取该模块的
   `get_errors_list`；不要盲目重试或全量生成。
4. 分开汇报 ECUC 配置改动与生成的 C/H/LSL 输出。
5. 每次会话结束都通过包装器发送 `{ "func": "shutdown_host" }`。

## 必须遵守

- 能使用 LGK-Vector 完成的 DaVinci ECUC 改动，不要手工改写 ARXML。
- `edit_file`、`import_dbc`、`update_project`、`auto_solve_errors`、
  `generate_code` 和 `shutdown_host` 必须各自单独发送，不能混在数组请求中。
- `auto_solve_errors` 必须先有最新错误列表、用户明确同意，并传入
  `confirmed:true`。
- `import_dbc` 或 `update_project` 前，保存并关闭 DaVinci GUI。
- 仅使用支持的函数：`inspect_ecuc_containers`、`find_module`、
  `find_module_template`、`get_param_definition`、`locate_container`、
  `edit_file`、`get_errors_list`、`auto_solve_errors`、`generate_code`、
  `update_project`、`import_dbc`、`shutdown_host`。

## 示例

```powershell
& "<skill-root>\Invoke-LGKVector.ps1" `
  -ProjectPath "D:\\Work\\Vehicle\\Cfg" `
  -Request '{"func":"inspect_ecuc_containers","module":"Com","container":"ComSignal"}'
```

跨 Agent 接入方式和工程工作规则见同目录的 `AGENTS.md`。
