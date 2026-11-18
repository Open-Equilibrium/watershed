import Cocoa
import FlutterMacOS
import XCTest

@testable import Liquid

class RunnerTests: XCTestCase {
  func testRefusedHelperShowsItsOwnMessage() {
    let refusal = helperRefusal(status: 1, said: Data("  Caps Lock is still held by Liquid\n".utf8))
    XCTAssertEqual(refusal.localizedDescription, "Caps Lock is still held by Liquid")

    let silent = helperRefusal(status: 3, said: Data())
    XCTAssertEqual(silent.localizedDescription, "the Liquid key helper failed with status 3")
  }

}
