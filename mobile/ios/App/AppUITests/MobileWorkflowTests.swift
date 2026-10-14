import XCTest

final class MobileWorkflowTests: XCTestCase {
    func testChatGPTAccountPanelLoadsNativeStorage() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launch()
        let scan = app.buttons.matching(NSPredicate(format: "label CONTAINS 'Scan & create'")).firstMatch
        XCTAssertTrue(scan.waitForExistence(timeout: 40), app.debugDescription)
        scan.tap()
        let sample = app.buttons["Try a sample scan"]
        if !sample.waitForExistence(timeout: 3) { scan.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap() }
        XCTAssertTrue(sample.waitForExistence(timeout: 10), app.debugDescription)
        let output = app.descendants(matching: .any).matching(NSPredicate(format: "value == 'Original scan + selectable text' OR label == 'Original scan + selectable text'")).firstMatch
        XCTAssertTrue(output.waitForExistence(timeout: 10), app.debugDescription)
        output.tap()
        let wheel = app.pickerWheels.firstMatch
        if wheel.waitForExistence(timeout: 2) {
            wheel.adjust(toPickerWheelValue: "AI-rebuilt digital document")
            if app.buttons["Done"].exists { app.buttons["Done"].tap() }
        } else {
            let option = app.descendants(matching: .any).matching(NSPredicate(format: "label == 'AI-rebuilt digital document'")).firstMatch
            XCTAssertTrue(option.waitForExistence(timeout: 5), app.debugDescription)
            option.tap()
        }
        let connect = app.buttons["Connect AI provider"]
        XCTAssertTrue(connect.waitForExistence(timeout: 10), app.debugDescription)
        // Scroll the scan dialog so the connection button is clear of its clipped lower edge.
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.7, dy: 0.8)).press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.7, dy: 0.4)))
        connect.tap()
        XCTAssertTrue(app.buttons["Continue with ChatGPT"].waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertTrue(app.staticTexts["Continue with ChatGPT to connect your plan."].waitForExistence(timeout: 15), app.debugDescription)
        XCTAssertFalse(app.buttons["Use ChatGPT plan"].isEnabled)
        let capture = XCTAttachment(screenshot: app.screenshot())
        capture.name = "ChatGPT native account storage loaded without signing in"
        capture.lifetime = .keepAlways
        add(capture)
        // No external sign-in or paid request is initiated by this smoke test.
        app.buttons["Close AI settings"].tap()
    }

    func testScanRecognizeReviewAndSave() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launch()
        let scan = app.buttons.matching(NSPredicate(format: "label CONTAINS 'Scan & create'")).firstMatch
        XCTAssertTrue(scan.waitForExistence(timeout: 40), app.debugDescription)
        scan.tap()
        let sample = app.buttons["Try a sample scan"]
        // WKWebView can finish a restored scroll/focus transition on the first tap.
        if !sample.waitForExistence(timeout: 3) { scan.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap() }
        XCTAssertTrue(sample.waitForExistence(timeout: 10), app.debugDescription)
        sample.tap()
        let process = app.buttons["Recognize text on device"]
        XCTAssertTrue(process.waitForExistence(timeout: 10), app.debugDescription)
        process.tap()
        let line = app.textViews.matching(NSPredicate(format: "label == 'Recognized line 1'")).firstMatch
        XCTAssertTrue(line.waitForExistence(timeout: 40), app.debugDescription)
        XCTAssertFalse((line.value as? String ?? "").isEmpty, app.debugDescription)
        let review = app.staticTexts["I reviewed the recognized text on every page."]
        review.tap()
        app.buttons["Save PDF"].tap()
        let title = app.staticTexts["Scanned document.pdf"]
        XCTAssertTrue(title.waitForExistence(timeout: 30), app.debugDescription)
        app.buttons["Document options"].tap()
        app.buttons.matching(NSPredicate(format: "label CONTAINS 'Select & copy page text'")).firstMatch.tap()
        let text = app.textViews["Selectable page text"]
        XCTAssertTrue(text.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertTrue((text.value as? String ?? "").lowercased().contains("make this page yours"), app.debugDescription)
        app.buttons["Close page text"].tap()
        let capture = XCTAttachment(screenshot: app.screenshot())
        capture.name = "Searchable scan created with on-device OCR"
        capture.lifetime = .keepAlways
        add(capture)
    }

    func testNativeDocumentPickerOpens() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launch()
        let open = app.buttons.matching(NSPredicate(format: "label CONTAINS 'Open a PDF'")).firstMatch
        XCTAssertTrue(open.waitForExistence(timeout: 40), app.debugDescription)
        open.tap()
        XCTAssertTrue(app.buttons["Browse"].waitForExistence(timeout: 10) || app.navigationBars["Recents"].exists || app.buttons["Cancel"].exists, app.debugDescription)
    }

    func testSampleFormAndNativeShare() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launch()
        let sample = app.buttons["Try a sample PDF"]
        XCTAssertTrue(sample.waitForExistence(timeout: 40), app.debugDescription)
        sample.tap()
        let title = app.staticTexts["Welcome to Tandem.pdf"]
        XCTAssertTrue(title.waitForExistence(timeout: 20), app.debugDescription)
        let fill = app.buttons.matching(NSPredicate(format: "label CONTAINS 'Fill'")).firstMatch
        XCTAssertTrue(fill.waitForExistence(timeout: 15), app.debugDescription)
        fill.tap()
        let field = app.textFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 10), app.debugDescription)
        field.tap()
        let value = "iPhone " + String(UUID().uuidString.prefix(8))
        field.typeText(value)
        app.buttons["Close form fields"].tap()
        XCTAssertTrue(app.staticTexts["Saved on device"].waitForExistence(timeout: 20))
        app.buttons.matching(NSPredicate(format: "label ENDSWITH 'Sign'")).firstMatch.tap()
        let pad = app.descendants(matching: .any).matching(NSPredicate(format: "label == 'Draw your signature'")).firstMatch
        XCTAssertTrue(pad.waitForExistence(timeout: 10), app.debugDescription)
        let left = pad.coordinate(withNormalizedOffset: CGVector(dx: 0.15, dy: 0.65))
        let middle = pad.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3))
        let right = pad.coordinate(withNormalizedOffset: CGVector(dx: 0.85, dy: 0.65))
        left.press(forDuration: 0.05, thenDragTo: middle)
        middle.press(forDuration: 0.05, thenDragTo: right)
        app.buttons["Use signature"].tap()
        let paper = app.descendants(matching: .any).matching(NSPredicate(format: "label == 'Page annotation surface'")).firstMatch
        XCTAssertTrue(paper.waitForExistence(timeout: 10), app.debugDescription)
        paper.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.82)).tap()
        app.buttons["Document options"].tap()
        app.buttons.matching(NSPredicate(format: "label CONTAINS 'Notes & marks on this page'")).firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Drawn signature"].waitForExistence(timeout: 15), app.debugDescription)
        app.buttons["Close annotations"].tap()
        let capture = XCTAttachment(screenshot: app.screenshot())
        capture.name = "Tandem PDF on iPhone"
        capture.lifetime = .keepAlways
        add(capture)
        app.buttons["Back to documents"].tap()
        let saved = app.buttons.matching(NSPredicate(format: "label CONTAINS 'Welcome to Tandem.pdf'")).firstMatch
        XCTAssertTrue(saved.waitForExistence(timeout: 10), app.debugDescription)
        app.terminate()
        app.launch()
        let reopened = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'PDF Welcome to Tandem.pdf'")).firstMatch
        XCTAssertTrue(reopened.waitForExistence(timeout: 20))
        app.swipeUp()
        reopened.tap()
        let reopenedFill = app.buttons.matching(NSPredicate(format: "label CONTAINS 'Fill'")).firstMatch
        XCTAssertTrue(reopenedFill.waitForExistence(timeout: 20), app.debugDescription)
        reopenedFill.tap()
        let reopenedField = app.textFields.firstMatch
        XCTAssertTrue(reopenedField.waitForExistence(timeout: 10))
        XCTAssertEqual(reopenedField.value as? String, value)
        app.buttons["Close form fields"].tap()
        app.buttons["Document options"].tap()
        app.buttons.matching(NSPredicate(format: "label CONTAINS 'Notes & marks on this page'")).firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Drawn signature"].waitForExistence(timeout: 15), app.debugDescription)
        app.buttons["Close annotations"].tap()
        let share = app.buttons.matching(NSPredicate(format: "label CONTAINS 'Share'")).firstMatch
        share.tap()
        // The system share sheet is the native boundary; don't send to a recipient.
        let saveToFiles = app.descendants(matching: .any).matching(NSPredicate(format: "label == 'Save to Files'")).firstMatch
        // A just-dismissed WKWebView dialog can consume the first tap while focus settles.
        if !saveToFiles.waitForExistence(timeout: 3) && share.isHittable { share.tap() }
        XCTAssertTrue(saveToFiles.waitForExistence(timeout: 20), app.debugDescription)
        let shareCapture = XCTAttachment(screenshot: app.screenshot())
        shareCapture.name = "Native PDF share sheet"
        shareCapture.lifetime = .keepAlways
        add(shareCapture)
    }
}
