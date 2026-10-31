enum MacPermission { screenRecording, accessibility }

/// Accessibility is reflected live by `AXIsProcessTrusted`, while the screen
/// capture preflight may only change after a relaunch, so it goes last.
const kMacGuidePermissionOrder = [
  MacPermission.accessibility,
  MacPermission.screenRecording,
];

const _kMaxGuideLaunchesPerBuild = 3;
const _kDone = 'done';

/// Comma separated names of the missing permissions for the core's admin
/// setup, which clears only those grants.
String macAdminSetupArg(List<MacPermission> missing) {
  final names = <String>[];
  for (final permission in missing) {
    final name = permission == MacPermission.screenRecording
        ? 'screen'
        : 'accessibility';
    if (!names.contains(name)) names.add(name);
  }
  return names.join(',');
}

String macPermissionSettingsUrl(MacPermission permission) {
  const prefix = 'x-apple.systempreferences:com.apple.preference.security?';
  switch (permission) {
    case MacPermission.screenRecording:
      return '${prefix}Privacy_ScreenCapture';
    case MacPermission.accessibility:
      return '${prefix}Privacy_Accessibility';
  }
}

String? macBundlePathFromExecutable(String executable) {
  final parts = executable.split('/');
  if (parts.length < 4) return null;
  final macos = parts[parts.length - 2];
  final contents = parts[parts.length - 3];
  final bundle = parts[parts.length - 4];
  if (macos != 'MacOS' || contents != 'Contents' || !bundle.endsWith('.app')) {
    return null;
  }
  return parts.sublist(0, parts.length - 3).join('/');
}

/// Arguments for `/bin/sh` that wait for this GUI to exit, restart the
/// per-user server agent so it picks up the new grants, then reopen the app.
/// Values are positional parameters so paths with spaces stay intact.
List<String> macRelaunchArgs({
  required String bundlePath,
  required int pid,
  required int uid,
  required String agentLabel,
}) {
  if (!bundlePath.startsWith('/') || !bundlePath.endsWith('.app')) {
    throw ArgumentError.value(bundlePath, 'bundlePath');
  }
  if (pid <= 0) throw ArgumentError.value(pid, 'pid');
  if (uid < 0) throw ArgumentError.value(uid, 'uid');
  if (agentLabel.isEmpty) throw ArgumentError.value(agentLabel, 'agentLabel');
  const script = 'n=0; while kill -0 "\$1" 2>/dev/null && [ "\$n" -lt 300 ]; '
      'do sleep 0.1; n=\$((n+1)); done; '
      'kill -0 "\$1" 2>/dev/null && exit 1; '
      'launchctl kickstart -k "gui/\$2/\$3" >/dev/null 2>&1 || true; '
      '/usr/bin/open "\$4"';
  return [
    '-c',
    script,
    'rustdesk-relaunch',
    '$pid',
    '$uid',
    agentLabel,
    bundlePath,
  ];
}

/// Walks the user through Screen Recording and Accessibility once per build
/// (at most a few launches), then restarts the app when both are granted.
class MacPermissionGuide {
  final String buildSeq;
  final List<MacPermission> required;
  final bool Function(MacPermission) isGranted;
  final void Function(MacPermission) request;
  /// One administrator prompt on the first launch of a build: installs the
  /// launchd daemon when missing and clears grants left by an earlier
  /// signature unless the updater already did.
  final Future<void> Function(List<MacPermission> missing) adminSetup;
  final void Function(MacPermission) openSettings;
  final String Function() readState;
  final void Function(String) writeState;
  /// Restart callbacks return false to defer, e.g. during a remote session.
  final bool Function() restart;
  final bool Function() restartAgent;

  MacPermission? _current;
  bool _awaitingRestart = false;
  bool _awaitingAgentRestart = false;

  MacPermissionGuide({
    required this.buildSeq,
    required this.required,
    required this.isGranted,
    required this.request,
    required this.adminSetup,
    required this.openSettings,
    required this.readState,
    required this.writeState,
    required this.restart,
    required this.restartAgent,
  });

  bool get active =>
      _current != null || _awaitingRestart || _awaitingAgentRestart;

  Future<void> start() async {
    if (buildSeq.isEmpty || required.isEmpty || active) return;
    final stored = readState().split(':');
    final sameBuild = stored.length == 2 && stored[0] == buildSeq;
    var launches = 0;
    if (sameBuild) {
      if (stored[1] == _kDone) return;
      launches = (int.tryParse(stored[1]) ?? 0).clamp(0, _kMaxGuideLaunchesPerBuild);
      if (launches >= _kMaxGuideLaunchesPerBuild) return;
    }
    final missing = _firstMissing();
    if (launches == 0) {
      // Fresh install, manual copy or update: install the daemon when missing
      // and, only if a grant is missing, clear grants of an older signature.
      await adminSetup(required.where((p) => !isGranted(p)).toList());
    }
    if (missing == null) {
      // Granted after an earlier prompt but outside this guide (for example
      // via the system "Quit & Reopen"): the agent still runs ungranted.
      // Keep the state pending until the restart happens so a deferred one
      // is retried on the next launch.
      if (launches > 0) {
        _awaitingAgentRestart = true;
        _tryRestartAgent();
      } else {
        writeState('$buildSeq:$_kDone');
      }
      return;
    }
    writeState('$buildSeq:${launches + 1}');
    _prompt(_firstMissing() ?? missing);
  }

  /// Returns true when a permission was newly granted, so callers can
  /// refresh the permission cards.
  bool tick() {
    if (_awaitingAgentRestart) {
      _tryRestartAgent();
      return false;
    }
    if (_awaitingRestart) {
      _tryRestart();
      return false;
    }
    final current = _current;
    if (current == null || !isGranted(current)) return false;
    final next = _firstMissing();
    if (next != null) {
      _prompt(next);
      return true;
    }
    _current = null;
    _awaitingRestart = true;
    _tryRestart();
    return true;
  }

  void _tryRestartAgent() {
    if (!restartAgent()) return;
    _awaitingAgentRestart = false;
    writeState('$buildSeq:$_kDone');
  }

  void _tryRestart() {
    if (!restart()) return;
    _awaitingRestart = false;
    writeState('$buildSeq:$_kDone');
  }

  MacPermission? _firstMissing() {
    for (final permission in required) {
      if (!isGranted(permission)) return permission;
    }
    return null;
  }

  void _prompt(MacPermission permission) {
    _current = permission;
    request(permission);
    openSettings(permission);
  }
}
