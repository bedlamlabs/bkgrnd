import XCTest
@testable import bkgrnd

final class PlaybackStallDetectorTests: XCTestCase {
  func testDoesNotReportBeforeTimeout() {
    var detector = PlaybackStallDetector(timeout: 15, minimumProgress: 0.25)
    detector.reset(at: Date(timeIntervalSince1970: 100), position: 0)

    XCTAssertFalse(
      detector.evaluate(
        at: Date(timeIntervalSince1970: 114.9),
        position: 0,
        intendsToPlay: true
      )
    )
  }

  func testReportsNoProgressAtTimeoutOnlyOnce() {
    var detector = PlaybackStallDetector(timeout: 15, minimumProgress: 0.25)
    detector.reset(at: Date(timeIntervalSince1970: 100), position: 0)

    XCTAssertTrue(
      detector.evaluate(
        at: Date(timeIntervalSince1970: 115),
        position: 0,
        intendsToPlay: true
      )
    )
    XCTAssertFalse(
      detector.evaluate(
        at: Date(timeIntervalSince1970: 130),
        position: 0,
        intendsToPlay: true
      )
    )
  }

  func testProgressRestartsTimeoutWindow() {
    var detector = PlaybackStallDetector(timeout: 15, minimumProgress: 0.25)
    detector.reset(at: Date(timeIntervalSince1970: 100), position: 0)

    XCTAssertFalse(
      detector.evaluate(
        at: Date(timeIntervalSince1970: 110),
        position: 0.5,
        intendsToPlay: true
      )
    )
    XCTAssertFalse(
      detector.evaluate(
        at: Date(timeIntervalSince1970: 124.9),
        position: 0.5,
        intendsToPlay: true
      )
    )
    XCTAssertTrue(
      detector.evaluate(
        at: Date(timeIntervalSince1970: 125),
        position: 0.5,
        intendsToPlay: true
      )
    )
  }

  func testIntentionalPauseSuspendsTimeout() {
    var detector = PlaybackStallDetector(timeout: 15, minimumProgress: 0.25)
    detector.reset(at: Date(timeIntervalSince1970: 100), position: 0)

    XCTAssertFalse(
      detector.evaluate(
        at: Date(timeIntervalSince1970: 120),
        position: 0,
        intendsToPlay: false
      )
    )
    XCTAssertFalse(
      detector.evaluate(
        at: Date(timeIntervalSince1970: 134.9),
        position: 0,
        intendsToPlay: true
      )
    )
    XCTAssertTrue(
      detector.evaluate(
        at: Date(timeIntervalSince1970: 135),
        position: 0,
        intendsToPlay: true
      )
    )
  }
}

final class PlaybackRecoveryPolicyTests: XCTestCase {
  func testAllowsOnlyOneRetryForCurrentURL() {
    var policy = PlaybackRecoveryPolicy()

    XCTAssertTrue(policy.claimRetry(for: "https://example.com/one"))
    XCTAssertFalse(policy.claimRetry(for: "https://example.com/one"))
  }

  func testResetAllowsNewUserAttemptForSameURL() {
    var policy = PlaybackRecoveryPolicy()
    XCTAssertTrue(policy.claimRetry(for: "https://example.com/one"))

    policy.reset()

    XCTAssertTrue(policy.claimRetry(for: "https://example.com/one"))
  }
}
