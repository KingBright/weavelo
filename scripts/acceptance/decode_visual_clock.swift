// Decode the redundant marker after arbitrary uniform native viewer scaling.
import AppKit
import Foundation
guard CommandLine.arguments.count == 2,
      let bitmap = NSBitmapImageRep(data: try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1])))
else { fatalError("Expected a clock fixture PNG") }
let width = bitmap.pixelsWide, height = bitmap.pixelsHigh
func rgb(_ x: Int, _ y: Int) -> NSColor? {
    guard x >= 0, y >= 0, x < width, y < height else { return nil }
    return bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB)
}
func marker(_ c: NSColor?, _ cyan: Bool) -> Bool {
    guard let c else { return false }
    return c.blueComponent > 0.65 && (cyan
        ? c.greenComponent > 0.65 && c.greenComponent - c.redComponent > 0.25
        : c.redComponent > 0.65 && c.redComponent - c.greenComponent > 0.25)
}
func runs(_ y: Int, _ cyan: Bool) -> [ClosedRange<Int>] {
    var output: [ClosedRange<Int>] = [], start: Int?
    for x in 0...width {
        if x < width && marker(rgb(x, y), cyan) { if start == nil { start = x } }
        else if let begin = start { if x - begin >= 3 { output.append(begin...(x - 1)) }; start = nil }
    }
    return output
}
func bit(_ x: Double, _ y: Double) -> Bool? {
    guard let c = rgb(Int(x.rounded()), Int(y.rounded())),
          abs(c.redComponent - c.greenComponent) < 0.2,
          abs(c.greenComponent - c.blueComponent) < 0.2 else { return nil }
    let luma = (c.redComponent + c.greenComponent + c.blueComponent) / 3
    return luma > 0.75 ? true : luma < 0.25 ? false : nil
}
var candidates = Set<UInt64>()
for y in stride(from: 0, to: height, by: 2) {
    for left in runs(y, false) {
        for right in runs(y, true) where right.lowerBound > left.upperBound + 100 {
            let x = Double(left.lowerBound + left.upperBound) / 2
            let scale = (Double(right.lowerBound + right.upperBound) / 2 - x) / 412
            var stamp: UInt64 = 0, valid = scale > 0.25 && scale < 10
            for index in 0..<64 where valid {
                let at = x + Double(14 + index * 6) * scale
                guard let on = bit(at, Double(y)), let inverse = bit(at, Double(y) + 18 * scale), on != inverse
                else { valid = false; break }
                if on { stamp |= UInt64(1) << index }
            }
            if valid { candidates.insert(stamp) }
        }
    }
}
guard candidates.count == 1, let stamp = candidates.first else {
    fputs("Clock marker missing, corrupted or ambiguous\n", stderr); exit(2)
}
print(String(data: try JSONSerialization.data(withJSONObject: ["draw_utc_ms": stamp,
    "width": width, "height": height]), encoding: .utf8)!)
