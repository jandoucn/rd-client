import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/consts.dart';
import 'package:flutter_hbb/desktop/widgets/mac_permission_guide.dart';
import 'package:flutter_hbb/models/platform_model.dart';
import 'package:flutter_hbb/utils/multi_window_manager.dart';
import 'package:flutter_hbb/utils/platform_channel.dart';
import 'package:url_launcher/url_launcher.dart';
import 'package:window_manager/window_manager.dart';

const _kMacPermissionGuideOption = 'mac-permission-guide';

/// Starts the post-update permission guide on macOS and returns it so the
/// home page timer can drive [MacPermissionGuide.tick].
Future<MacPermissionGuide?> startMacPermissionGuide() async {
  if (!isMacOS) return null;
  try {
    final metadata = jsonDecode(await bind.mainGetUpdateMetadata());
    final buildSeq = metadata is Map ? '${metadata['build_seq'] ?? ''}' : '';
    final guide = MacPermissionGuide(
      buildSeq: buildSeq,
      required: bind.isOutgoingOnly() ? const [] : kMacGuidePermissionOrder,
      isGranted: (p) => _query(p, prompt: false),
      request: (p) => _query(p, prompt: true),
      adminSetup: (missing) async {
        final result = await bind.mainGetCommon(
            key: 'mac-admin-setup:${macAdminSetupArg(missing)}');
        debugPrint('mac permission guide admin setup: $result');
      },
      openSettings: (p) =>
          launchUrl(Uri.parse(macPermissionSettingsUrl(p))).ignore(),
      readState: () =>
          bind.mainGetLocalOption(key: _kMacPermissionGuideOption),
      writeState: (v) => bind
          .mainSetLocalOption(key: _kMacPermissionGuideOption, value: v)
          .ignore(),
      restart: () {
        if (_sessionActive()) return false;
        _relaunch().ignore();
        return true;
      },
      restartAgent: () {
        if (_sessionActive()) return false;
        _kickstartAgent().ignore();
        return true;
      },
    );
    await guide.start();
    return guide;
  } catch (e) {
    debugPrint('mac permission guide unavailable: $e');
    return null;
  }
}

bool _query(MacPermission permission, {required bool prompt}) {
  switch (permission) {
    case MacPermission.screenRecording:
      return bind.mainIsCanScreenRecording(prompt: prompt);
    case MacPermission.accessibility:
      return bind.mainIsProcessTrusted(prompt: prompt);
  }
}

// Incoming sessions live in the per-user server, not this window, so they
// are probed over IPC. The probe is async; until a fresh "idle" answer
// arrives the guide treats the machine as busy and retries on the next tick.
bool _incomingActive = true;
bool _probing = false;

void _refreshIncomingActive() {
  if (_probing) return;
  _probing = true;
  bind
      .mainGetCommon(key: 'mac-has-active-incoming-connections')
      .then((v) => _incomingActive = v != 'false')
      .catchError((_) => _incomingActive = true)
      .whenComplete(() => _probing = false);
}

bool _sessionActive() {
  final busy = _incomingActive ||
      rustDeskWinManager.getActiveWindows().any((id) => id != kMainWindowId);
  _incomingActive = true;
  _refreshIncomingActive();
  return busy;
}

String _agentLabel() => 'com.carriez.${bind.mainGetAppNameSync()}_server';

Future<int> _uid() async {
  final id = await Process.run('/usr/bin/id', ['-u']);
  return int.tryParse(id.stdout.toString().trim()) ?? -1;
}

Future<void> _kickstartAgent() async {
  try {
    final uid = await _uid();
    if (uid < 0) return;
    await Process.run(
        '/bin/launchctl', ['kickstart', '-k', 'gui/$uid/${_agentLabel()}']);
  } catch (e) {
    debugPrint('mac permission guide agent restart failed: $e');
  }
}

Future<void> _relaunch() async {
  try {
    final bundle = macBundlePathFromExecutable(Platform.resolvedExecutable);
    if (bundle == null) return;
    final args = macRelaunchArgs(
      bundlePath: bundle,
      pid: pid,
      uid: await _uid(),
      agentLabel: _agentLabel(),
    );
    await Process.start('/bin/sh', args, mode: ProcessStartMode.detached);
    // A relaunch is not a user quit: keep the device online.
    await bind.mainSetCommon(key: 'mac-programmatic-quit', value: 'Y');
  } catch (e) {
    debugPrint('mac permission guide relaunch failed: $e');
    return;
  }
  await rustDeskWinManager.closeAllSubWindows();
  await windowManager.setPreventClose(false);
  await windowManager.close();
  // Same workaround as onActiveWindowChanged: terminate needs a retry loop.
  periodic_immediate(
      const Duration(milliseconds: 30), RdPlatformChannel.instance.terminate);
}
