import AppKit

/// What the menu bar item says when it shows usage: the tightest limit among
/// the accounts on the rail.
///
/// **The rail's own figure, not a new one.** Each account contributes the
/// limit its ring draws (`headlineWindow`, honouring a pinned limit) and the
/// fullest of those wins, so the menu bar never disagrees with the ring it
/// sums up. Left out: an account with no reading, and a ring Pulse inferred —
/// a balance measured against a watched peak or a typed budget — since the
/// menu bar has no room to say "estimate". The tooltip names the account.
struct MenuBarReading: Equatable {
    let account: AccountKey
    let window: UsageWindow
    /// Past the reader's warning line, or spent. Drawn in red.
    let isAlert: Bool

    static func tightest(
        among accounts: [AccountKey],
        usage: (AccountKey) -> ProviderUsage,
        pinned: (AccountKey) -> String?,
        warningAt threshold: Double
    ) -> MenuBarReading? {
        var best: MenuBarReading?
        for account in accounts {
            let reading = usage(account)
            if case .unavailable = reading.state { continue }
            guard let window = reading.headlineWindow(preferring: pinned(account)),
                  window.estimate == nil
            else { continue }
            // Strictly greater, so a tie goes to the account earlier on the
            // rail — the order the reader chose.
            if let best, window.usedFraction <= best.window.usedFraction { continue }
            best = MenuBarReading(
                account: account,
                window: window,
                isAlert: UsageTint.isSpent(window) || window.usedFraction >= threshold
            )
        }
        return best
    }

    func text(remaining: Bool) -> String {
        window.percentText(remaining: remaining)
    }
}

extension MenuBarReading {
    /// Puts a reading on the status item's button: Pulse's mark alone when
    /// there is none, else the account's mark and its figure, red past the
    /// warning line. The tooltip names the account and the limit.
    @MainActor
    static func draw(_ reading: MenuBarReading?, remaining: Bool, label: String?, on button: NSStatusBarButton) {
        guard let reading else {
            button.image = NSImage(systemSymbolName: "chart.pie.fill", accessibilityDescription: "Pulse")
            button.image?.isTemplate = true
            button.attributedTitle = NSAttributedString()
            button.imagePosition = .imageOnly
            button.toolTip = "Pulse"
            return
        }

        // A copy at menu bar size: the store's image is shared, and large.
        let mark = (LobeIconStore.image(for: reading.account.provider)?.copy() as? NSImage)
            ?? NSImage(systemSymbolName: "chart.pie.fill", accessibilityDescription: nil)
        mark?.size = NSSize(width: 15, height: 15)
        mark?.isTemplate = true
        button.image = mark
        button.imagePosition = .imageLeading

        var attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedDigitSystemFont(ofSize: NSFont.systemFontSize, weight: .medium),
        ]
        if reading.isAlert { attributes[.foregroundColor] = NSColor.systemRed }
        button.attributedTitle = NSAttributedString(
            string: " " + reading.text(remaining: remaining),
            attributes: attributes
        )

        let window = reading.window
        let figure = remaining
            ? String.localized("\(window.percentText(remaining: true)) left, \(window.name)")
            : String.localized("\(window.percentText) used, \(window.name)")
        button.toolTip = [label, figure].compactMap { $0 }.joined(separator: "\n")
    }
}
