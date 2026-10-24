// CI-only: hold the display setter alive around the unchanged native smoke.
import AppKit
import CoreGraphics
import Darwin

func rect(_ r: NSRect) -> [Double] {
    [Double(r.origin.x), Double(r.origin.y), Double(r.width), Double(r.height)]
}
func modeRecord(_ m: CGDisplayMode) -> [String: Any] {
    ["id": m.ioDisplayModeID, "width": m.width, "height": m.height,
     "pixelWidth": m.pixelWidth, "pixelHeight": m.pixelHeight,
     "refreshRate": m.refreshRate, "ioFlags": m.ioFlags,
     "desktopGUIUsable": m.isUsableForDesktopGUI()]
}
func sameMode(_ a: CGDisplayMode, _ b: CGDisplayMode) -> Bool {
    a.ioDisplayModeID == b.ioDisplayModeID && a.width == b.width &&
    a.height == b.height && a.pixelWidth == b.pixelWidth &&
    a.pixelHeight == b.pixelHeight && a.ioFlags == b.ioFlags &&
    a.refreshRate == b.refreshRate
}
func screensRecord() -> [[String: Any]] {
    NSScreen.screens.map { s in
        ["display": (s.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value ?? 0,
         "frame": rect(s.frame), "visibleFrame": rect(s.visibleFrame),
         "backingScaleFactor": Double(s.backingScaleFactor)]
    }.sorted { ($0["display"] as! UInt32) < ($1["display"] as! UInt32) }
}
func screenFor(_ display: CGDirectDisplayID) -> NSScreen? {
    NSScreen.screens.first {
        ($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value == display
    }
}
func geometryEqual(_ a: [[String: Any]], _ b: [[String: Any]]) -> Bool {
    NSDictionary(dictionary: ["screens": a]).isEqual(to: ["screens": b])
}
var interrupted = false
signal(SIGTERM, SIG_IGN)
signal(SIGINT, SIG_IGN)
let term = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
let interrupt = DispatchSource.makeSignalSource(signal: SIGINT, queue: .main)
term.setEventHandler { interrupted = true }
interrupt.setEventHandler { interrupted = true }
term.resume()
interrupt.resume()
func pump(_ seconds: Double) {
    let end = Date().addingTimeInterval(seconds)
    while Date() < end {
        _ = RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.05))
    }
}
func settle(_ seconds: Double, _ predicate: () -> Bool) -> Bool {
    let end = Date().addingTimeInterval(seconds)
    var consecutive = 0
    repeat {
        pump(0.1)
        consecutive = predicate() ? consecutive + 1 : 0
        if consecutive >= 3 { return true }
    } while Date() < end
    return false
}

let output = CommandLine.arguments[1]
let python = CommandLine.arguments[2]
let driver = CommandLine.arguments[3]
var report: [String: Any] = ["schema": 1, "scope": "CI-only-display-smoke-wrapper",
    "smokeChildLaunched": false, "nativeAcceptance": "unchanged native-smoke.json only", "switchAttempts": 0,
    "restoration": "unverified", "result": "uninitialized"]
var exitCode: Int32 = 1
var reportWriteFailed = false
func checkpoint() {
    do {
        let data = try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
        try data.write(to: URL(fileURLWithPath: output), options: .atomic)
    } catch { reportWriteFailed = true; exitCode = 3 }
}
// Keep Process alive through deferred display restoration and explicit process exit.
// Failure paths never release a running Process or depend on its deinitializer.
let smokeSupervisor = Process()
func runSmoke() {
    if interrupted { report["result"] = "cancelled_before_smoke"; return }
    let child = smokeSupervisor
    child.executableURL = URL(fileURLWithPath: python)
    let receipt = URL(fileURLWithPath: output).deletingLastPathComponent().appendingPathComponent("child.json").path
    let root = URL(fileURLWithPath: driver).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().path
    child.arguments = [driver, "--child", receipt, "2400", "/usr/bin/env", "node", root + "/scripts/ci/smoke.mjs"]
    report["smokeCommand"] = ["node", "scripts/ci/smoke.mjs"]
    report["result"] = "launching_smoke"
    checkpoint()
    do { try child.run() }
    catch { report["result"] = "smoke_launch_error"; report["error"] = String(describing: error); return }
    report["smokeChildLaunched"] = true
    report["childSupervisorPID"] = child.processIdentifier
    checkpoint()
    let end = Date().addingTimeInterval(2410)
    while child.isRunning {
        pump(0.05)
        if interrupted || Date() >= end { break }
    }
    let deadlineExceeded = Date() >= end
    // Cancellation/deadline wins even when pump also delivered child completion.
    if interrupted || deadlineExceeded {
        report["cancelled"] = interrupted
        report["deadlineExceeded"] = deadlineExceeded
        report["result"] = "smoke_abandoned_cleanup_delegated"
        report["smokeSupervisorState"] = child.isRunning ? "running_abandoned" : "completed_at_stop"
        report["cleanupOwner"] = "outer_python_supervisor"
        report["cleanup"] = "unverified_until_outer_report"
        report["cleanupVerified"] = false
        exitCode = 1
        // Outer Python tracks this helper's descendants while live and cleans
        // them after our nonzero exit. Its sampling is not OS containment.
        checkpoint()
        return
    }
    report["result"] = "smoke_failed"
    // Fresh status read after pumping; never access terminal properties while live.
    guard !child.isRunning else {
        report["result"] = "smoke_state_unverified"
        checkpoint()
        return
    }
    report["smokeSupervisorExit"] = child.terminationStatus
    report["smokeSupervisorReason"] = child.terminationReason.rawValue
    if child.terminationStatus != 0 { exitCode = child.terminationStatus }
    if let data = try? Data(contentsOf: URL(fileURLWithPath: receipt)),
       let status = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
        report["smokeChild"] = status
        if child.terminationReason == .exit && child.terminationStatus == 0 &&
           (status["exit"] as? Int) == 0 && (status["cleanupVerified"] as? Bool) == true {
            report["result"] = "smoke_child_exit_zero"; exitCode = 0
        }
    }
    // Only smoke.mjs validates native acceptance; this report records process state.
    checkpoint()
}
func experiment() {
    _ = NSApplication.shared
    NSApp.setActivationPolicy(.prohibited)
    pump(0.3)
    let display = CGMainDisplayID()
    let before = screensRecord()
    report["initialScreens"] = before
    report["display"] = display
    guard let original = CGDisplayCopyDisplayMode(display),
          let modes = CGDisplayCopyAllDisplayModes(display, nil) as? [CGDisplayMode],
          let screen = screenFor(display) else {
        report["result"] = "inventory_unavailable"; return
    }
    report["originalMode"] = modeRecord(original)
    report["enumeratedModeCount"] = modes.count
    guard modes.count <= 512 else {
        report["result"] = "mode_inventory_exceeds_diagnostic_bound"; return
    }
    report["allModes"] = modes.map(modeRecord)
    report["displayActive"] = CGDisplayIsActive(display) != 0
    report["displayOnline"] = CGDisplayIsOnline(display) != 0
    report["mirrored"] = CGDisplayIsInMirrorSet(display) != 0
    // Refuse topology changes: this experiment supports exactly one unmirrored display.
    guard before.count == 1, CGDisplayIsInMirrorSet(display) == 0,
          CGDisplayIsActive(display) != 0, CGDisplayIsOnline(display) != 0,
          original.isUsableForDesktopGUI() else {
        report["result"] = "unsupported_display_topology"; return
    }
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1080, height: 600),
        styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
        backing: .buffered, defer: false, screen: screen)
    window.isReleasedWhenClosed = false
    window.orderFrontRegardless()
    defer { window.orderOut(nil); window.close() }
    pump(0.3)
    func nativeFit() -> Bool {
        guard let s = screenFor(display), let view = window.contentView else { return false }
        let insetW = view.bounds.width - window.contentLayoutRect.width
        let insetH = view.bounds.height - window.contentLayoutRect.height
        report["nativeProbe"] = ["contentBounds": rect(view.bounds),
            "contentLayoutRect": rect(window.contentLayoutRect),
            "measuredInset": [Double(insetW), Double(insetH)],
            "requiredVisible": [Double(1080 + insetW + 32), Double(760 + insetH + 32)],
            "visibleFrame": rect(s.visibleFrame)]
        return insetW.isFinite && insetH.isFinite && insetW >= 0 && insetH >= 0 &&
            insetW <= 100 && insetH <= 100 &&
            s.visibleFrame.width >= 1080 + insetW + 32 &&
            s.visibleFrame.height >= 760 + insetH + 32
    }
    if nativeFit() {
        report["restoration"] = "not_needed"
        report["switchedMode"] = modeRecord(original)
        report["switchedScreens"] = screensRecord()
        runSmoke(); return
    }
    // Mode dimensions are ONLY a conservative ranking heuristic, not logical-point evidence.
    // Limit to non-scaled modes and current scale 1; actual NSScreen verification is mandatory.
    let candidates = modes.filter {
        $0.isUsableForDesktopGUI() && !sameMode($0, original) &&
        $0.width >= 1280 && $0.height >= 900 && $0.width <= 1920 && $0.height <= 1200 &&
        $0.width == $0.pixelWidth && $0.height == $0.pixelHeight &&
        $0.ioFlags == original.ioFlags && $0.refreshRate == original.refreshRate
    }.sorted {
        if $0.width * $0.height != $1.width * $1.height {
            return $0.width * $0.height < $1.width * $1.height
        }
        return $0.ioDisplayModeID < $1.ioDisplayModeID
    }
    guard screen.backingScaleFactor == 1, let chosen = candidates.first else {
        report["result"] = "no_adequate_enumerated_candidate_under_conservative_policy"; return
    }
    if interrupted { report["result"] = "interrupted_before_switch"; return }
    report["selectedMode"] = modeRecord(chosen)
    var attempted = false
    // Hold the exact original object and caller until explicit restoration/readback completes.
    defer {
        if attempted {
            let restore = CGDisplaySetDisplayMode(display, original, nil)
            report["restoreCGError"] = restore.rawValue
            let restored = settle(6) {
                guard let actual = CGDisplayCopyDisplayMode(display) else { return false }
                return sameMode(actual, original) && geometryEqual(screensRecord(), before)
            }
            report["restoredScreens"] = screensRecord()
            if let actual = CGDisplayCopyDisplayMode(display) {
                report["restoredMode"] = modeRecord(actual)
            }
            report["restoration"] = restore == .success && restored ? "verified" : "FAILED"
            if restore != .success || !restored {
                report["experimentResultBeforeRestoreFailure"] = report["result"]
                report["result"] = "restoration_failure"; exitCode = 2
            }
            checkpoint()
        }
    }
    attempted = true // Set BEFORE API call: even error/partial switches require restoration.
    report["switchAttempts"] = 1
    checkpoint()
    let switched = CGDisplaySetDisplayMode(display, chosen, nil)
    report["switchCGError"] = switched.rawValue
    guard switched == .success else { report["result"] = "switch_rejected"; return }
    let verified = settle(6) {
        guard let actual = CGDisplayCopyDisplayMode(display) else { return false }
        return !interrupted && sameMode(actual, chosen) && nativeFit()
    }
    report["switchedScreens"] = screensRecord()
    if let actual = CGDisplayCopyDisplayMode(display) { report["switchedMode"] = modeRecord(actual) }
    if interrupted { report["result"] = "interrupted_after_switch"; return }
    guard verified else { report["result"] = "candidate_insufficient_or_unsettled"; return }
    // Recheck before launching the smoke while this exact setter stays alive.
    pump(0.5)
    guard !interrupted, let actual = CGDisplayCopyDisplayMode(display), sameMode(actual, chosen), nativeFit() else {
        report["result"] = "candidate_readback_lost"; return
    }
    runSmoke()
}
experiment()
report["finalScreens"] = screensRecord()
if interrupted {
    report["cancelled"] = true
    // Preserve the specific abandonment/restoration failure receipt if already failed.
    if exitCode == 0 {
        report["result"] = "cancelled_after_smoke"
        exitCode = 1
    }
}
checkpoint()
withExtendedLifetime(smokeSupervisor) {
    exit(reportWriteFailed ? 3 : exitCode)
}
