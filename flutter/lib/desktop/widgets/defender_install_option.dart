import 'package:flutter/material.dart';

bool defenderConsentFromOptions(Map<dynamic, dynamic> options) =>
    options['DEFENDER_EXCLUSIONS'] == '1';

String defenderInstallToken(bool selected) =>
    selected ? ' defender=1' : ' defender=0';

class DefenderInstallOption extends StatelessWidget {
  final bool selected;
  final bool enabled;
  final ValueChanged<bool> onChanged;

  const DefenderInstallOption({
    Key? key,
    required this.selected,
    required this.enabled,
    required this.onChanged,
  }) : super(key: key);

  @override
  Widget build(BuildContext context) => CheckboxListTile(
        key: const ValueKey('defender-file-exclusions'),
        contentPadding: EdgeInsets.zero,
        controlAffinity: ListTileControlAffinity.leading,
        value: selected,
        onChanged: enabled ? (value) => onChanged(value ?? false) : null,
        title: const Text('为 RustDesk 配置 Defender 文件排除项'),
        subtitle: const Text('保留主程序和管理代理两个精确 EXE，并增加安装目录、'
            'ManagementAgent、Client 三个目录排除项。受控文件夹访问已启用时，'
            '只为这两个 EXE 添加允许应用。配置失败不阻止安装；不能保证不被拦截，'
            '不覆盖 360，360 信任区仍需手动配置。'),
      );
}
