import 'package:flutter/widgets.dart';

/// macOS: closing the window keeps the device online, while an explicit quit
/// (Cmd+Q or Dock "Quit") takes it offline until the app is opened again.
///
/// Going offline happens in the native `applicationWillTerminate`, because
/// Flutter routes exit requests to whichever engine registered last (a remote
/// window, once one was opened). This side only restores the state on launch.
class MacQuitOffline {
  final Future<void> Function(String key) setCommon;

  MacQuitOffline({required this.setCommon});

  void onLaunch() {
    setCommon('mac-restore-after-quit').catchError((Object e) {
      debugPrint('restore after quit failed: $e');
    });
  }
}

bool _installed = false;

/// Restores an explicit-quit offline state once for the main window.
void installMacQuitOffline(Future<void> Function(String key) setCommon) {
  if (_installed) return;
  _installed = true;
  MacQuitOffline(setCommon: setCommon).onLaunch();
}
