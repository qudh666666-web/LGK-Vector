# EXE 自检

这个目录给没有 Rust 编译环境的使用者验证发布包。

1. 完整解压 ZIP，不能只复制单个 `lgk-vector.exe`；`lgk-vector` 目录中的 CLI、Host、包装脚本和配对清单必须保持原来的相对位置。
2. 双击 `一键测试EXE.cmd`。
3. 看到 `"valid": true` 和 `LGK-Vector EXE self-test passed.` 即表示两只 EXE、中文/空格路径、本地 ECUC 查询、模板缓存和 Host 正常关闭均通过。

自检使用运行时创建的公开合成数据，不包含客户 DPA、DBC、ARXML 或 Vector 文件，也不会启动 DaVinci。它不能代替目标电脑上的 DaVinci/SIP/许可证集成验证。

接入真实工程时，在 DaVinci `Cfg` 目录创建 `lgk-vector.json`：

```json
{
  "tool_path": "D:\\VectorSIP"
}
```

安装、使用和 Agent 接入见包根目录的 `README.md` 与 `lgk-vector/SKILL.md`。
