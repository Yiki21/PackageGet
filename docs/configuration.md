# Configuration

Updater使用单一配置schema。配置文件位于平台用户配置目录下的`updater/config.json`；Linux通常对应`~/.config/updater/config.json`。

## Schema

```json
{
  "managers": [
    {
      "id": "builtin:cargo",
      "executable": "/home/user/.cargo/bin/cargo",
      "settings": {}
    },
    {
      "id": "builtin:go",
      "executable": null,
      "settings": {
        "go_bin_dir": "/home/user/.local/bin"
      }
    }
  ],
  "appearance": "system",
  "notifications_enabled": false
}
```

- `managers`按稳定`ManagerId`保存启用的manager；同一ID不能重复。
- `executable`为可选自定义可执行文件路径；`null`表示使用默认命令发现规则。Settings可随时选择新路径或恢复为`null`。对于APT、DNF、Pacman和Zypper，该路径只用于availability与只读查询；特权写操作始终经由固定的`/usr/libexec/updater-system-helper`执行发行版标准系统命令，不能把自定义路径提升为root。
- `settings`必须是JSON object，由对应manager定义和校验；core不解释或记录其中可能包含的敏感值。
- `appearance`支持`system`、`light`、`dark`和`high_contrast`。
- `notifications_enabled`控制原生完成/失败通知。

未知但格式合法的第三方manager会保留在`managers`中。当前catalog中缺少该manager只影响运行时可用性，不会在保存Settings时删除它。

缺少必需字段、包含未知顶层字段或使用其他结构的文件会返回配置错误且不会被自动覆盖。Updater会显示启动恢复界面：`Retry`使用同一个严格loader再次读取；`Open Config Folder`只打开配置目录；`Reset Configuration`仅在二次确认后重新检测manager，并用默认应用设置原子替换现有文件。取消或打开目录都不会修改`config.json`，reset失败时仍保留并显示最初的load error。

## 写入语义

保存前会验证重复ID和settings类型。对于当前平台支持、已注册且设置了自定义`executable`的manager，Settings还会调用对应manager的availability检查，验证普通文件/执行权限、manager settings和version command；任一检查失败时不会写入文件，并在对应manager行显示失败原因。使用默认命令发现规则、当前平台不支持或当前build未注册的manager不会被该检查阻断，其配置仍原样保留。

Linux系统manager的写操作另受Updater Polkit helper约束。helper不读取这里的`executable`或任意manager settings，只接受固定动作、固定manager ID和严格校验的package名称；密码输入与认证结果完全由当前桌面Polkit authentication agent处理。

通过验证后，Updater先在同一目录写入临时文件并执行flush/sync，再使用rename替换`config.json`；验证或写入失败不会先截断现有配置。
