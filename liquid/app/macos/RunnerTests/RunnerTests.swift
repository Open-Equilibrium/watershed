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

  func testHelperWritingMoreThanAPipeHoldsIsReportedNotTimedOut() throws {
    let directory = FileManager.default.temporaryDirectory
      .appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let helper = directory.appendingPathComponent("helper")
    try """
    #!/bin/sh
    echo 'Caps Lock is still held by Liquid' >&2
    yes e | head -c 1048576 >&2
    exit 1

    """.write(to: helper, atomically: true, encoding: .utf8)
    try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: helper.path)

    let started = Date()
    XCTAssertThrowsError(try runHelper("--remove", at: helper)) { error in
      XCTAssertTrue(
        error.localizedDescription.hasPrefix("Caps Lock is still held by Liquid"),
        error.localizedDescription)
    }
    XCTAssertLessThan(Date().timeIntervalSince(started), 5)
  }
}
