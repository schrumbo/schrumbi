import Cocoa
import CoreGraphics
let a = CommandLine.arguments
func wins(_ pid: Int32) -> [[String: Any]] {
    let l = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
    return l.filter { ($0[kCGWindowOwnerPID as String] as? Int32) == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }
}
func src() -> CGEventSource? { CGEventSource(stateID: .hidSystemState) }
func activate(_ pid: Int32) {
    NSRunningApplication(processIdentifier: pid)?.activate(options: [.activateIgnoringOtherApps]); usleep(150000)
    let top = (CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]).first { ($0[kCGWindowLayer as String] as? Int) == 0 }
    if (top?[kCGWindowOwnerPID as String] as? Int32) != pid { print("refused: pid \(pid) is not frontmost"); exit(3) }
}
let flagsMap: [String: CGEventFlags] = ["cmd": .maskCommand, "shift": .maskShift, "alt": .maskAlternate, "ctrl": .maskControl]
func flags(_ s: String) -> CGEventFlags { var f = CGEventFlags(); for p in s.split(separator: "+") { if let v = flagsMap[String(p)] { f.insert(v) } }; return f }
switch a[1] {
case "list":
    for w in wins(Int32(a[2])!) { let b = w[kCGWindowBounds as String] as! [String: Any]; print(w[kCGWindowNumber as String]!, b["X"]!, b["Y"]!, b["Width"]!, b["Height"]!, w[kCGWindowName as String] ?? "") }
case "click":
    let pid = Int32(a[2])!; activate(pid)
    let w = wins(pid).first!; let b = w[kCGWindowBounds as String] as! [String: Any]
    let p = CGPoint(x: (b["X"] as! CGFloat) + CGFloat(Double(a[3])!), y: (b["Y"] as! CGFloat) + CGFloat(Double(a[4])!))
    let n = a.count > 5 ? Int(a[5])! : 1
    let btnDown: CGEventType = (a.count > 6 && a[6] == "right") ? .rightMouseDown : .leftMouseDown
    let btnUp: CGEventType = (a.count > 6 && a[6] == "right") ? .rightMouseUp : .leftMouseUp
    let btn: CGMouseButton = (a.count > 6 && a[6] == "right") ? .right : .left
    CGEvent(mouseEventSource: src(), mouseType: .mouseMoved, mouseCursorPosition: p, mouseButton: .left)?.post(tap: .cghidEventTap); usleep(60000)
    for i in 1...n {
        let d = CGEvent(mouseEventSource: src(), mouseType: btnDown, mouseCursorPosition: p, mouseButton: btn)!; if a.count > 7, let f = flagsMap[a[7]] { d.flags = f }; d.setIntegerValueField(.mouseEventClickState, value: Int64(i)); d.post(tap: .cghidEventTap); usleep(30000)
        let u = CGEvent(mouseEventSource: src(), mouseType: btnUp, mouseCursorPosition: p, mouseButton: btn)!; if a.count > 7, let f = flagsMap[a[7]] { u.flags = f }; u.setIntegerValueField(.mouseEventClickState, value: Int64(i)); u.post(tap: .cghidEventTap); usleep(60000)
    }
case "move":
    let pid = Int32(a[2])!; let w = wins(pid).first!; let b = w[kCGWindowBounds as String] as! [String: Any]
    let p = CGPoint(x: (b["X"] as! CGFloat) + CGFloat(Double(a[3])!), y: (b["Y"] as! CGFloat) + CGFloat(Double(a[4])!))
    CGEvent(mouseEventSource: src(), mouseType: .mouseMoved, mouseCursorPosition: p, mouseButton: .left)?.post(tap: .cghidEventTap)
case "key":
    let pid = Int32(a[2])!; activate(pid)
    let code = CGKeyCode(a[3])!; let f = a.count > 4 ? flags(a[4]) : []
    let d = CGEvent(keyboardEventSource: src(), virtualKey: code, keyDown: true)!; d.flags = f; d.post(tap: .cghidEventTap); usleep(20000)
    let u = CGEvent(keyboardEventSource: src(), virtualKey: code, keyDown: false)!; u.flags = f; u.post(tap: .cghidEventTap)
case "type":
    let pid = Int32(a[2])!; activate(pid)
    let alt: [Character: CGKeyCode] = ["[":23,"]":22,"{":28,"}":25,"|":26,"~":45,"@":37]
    let map: [Character: (CGKeyCode, Bool)] = [">":(50,true),"<":(50,false),"a":(0,false),"s":(1,false),"d":(2,false),"f":(3,false),"h":(4,false),"g":(5,false),"y":(6,false),"x":(7,false),"c":(8,false),"v":(9,false),"b":(11,false),"q":(12,false),"w":(13,false),"e":(14,false),"r":(15,false),"z":(16,false),"t":(17,false),"1":(18,false),"2":(19,false),"3":(20,false),"4":(21,false),"6":(22,false),"5":(23,false),"9":(25,false),"7":(26,false),"8":(28,false),"0":(29,false),"o":(31,false),"u":(32,false),"i":(34,false),"p":(35,false),"l":(37,false),"j":(38,false),"k":(40,false),"n":(45,false),"m":(46,false),".":(47,false),",":(43,false),"-":(44,false),"#":(42,false),"+":(30,false),":":(47,true),"?":(27,true),"=":(29,true),"!":(18,true),"$":(21,true),"/":(26,true),"(":(28,true),")":(25,true),"_":(44,true),"*":(30,true)," ":(49,false),"\n":(36,false)]
    for ch in a[3] {
        if let code = alt[ch] { let d = CGEvent(keyboardEventSource: src(), virtualKey: code, keyDown: true)!; d.flags = .maskAlternate; d.post(tap: .cghidEventTap); usleep(12000); let u = CGEvent(keyboardEventSource: src(), virtualKey: code, keyDown: false)!; u.flags = .maskAlternate; u.post(tap: .cghidEventTap); usleep(12000); continue }
        var key = map[ch]; var shift = false
        if key == nil, let l = ch.lowercased().first, let k = map[l] { key = k; shift = true }
        guard let (code, s2) = key else { continue }
        let f: CGEventFlags = (shift || s2) ? .maskShift : []
        let d = CGEvent(keyboardEventSource: src(), virtualKey: code, keyDown: true)!; d.flags = f; d.post(tap: .cghidEventTap); usleep(12000)
        let u = CGEvent(keyboardEventSource: src(), virtualKey: code, keyDown: false)!; u.flags = f; u.post(tap: .cghidEventTap); usleep(12000)
    }
case "wheel":
    let pid = Int32(a[2])!; activate(pid); let w = wins(pid).first!; let b = w[kCGWindowBounds as String] as! [String: Any]
    let p = CGPoint(x: (b["X"] as! CGFloat) + CGFloat(Double(a[3])!), y: (b["Y"] as! CGFloat) + CGFloat(Double(a[4])!))
    CGEvent(mouseEventSource: src(), mouseType: .mouseMoved, mouseCursorPosition: p, mouseButton: .left)?.post(tap: .cghidEventTap); usleep(60000)
    let n = Int(a[5])!; let f = a.count > 6 ? flags(a[6]) : []
    for _ in 0..<abs(n) { let e = CGEvent(scrollWheelEvent2Source: src(), units: .line, wheelCount: 1, wheel1: Int32(n > 0 ? 3 : -3), wheel2: 0, wheel3: 0)!; e.flags = f; e.post(tap: .cghidEventTap); usleep(40000) }
default: break
}
