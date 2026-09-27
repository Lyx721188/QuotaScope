import Foundation
import Testing
@testable import Pulse

/// Which ring the menu bar item speaks for: the rail's own headline figures,
/// the fullest winning, and nothing an unavailable or inferred ring says.
@Suite("Menu bar reading")
struct MenuBarReadingTests {
    private let claude = AccountKey(.claudeCode)
    private let codex = AccountKey(.codex)
    private let cursor = AccountKey(.cursor)

    private func window(_ id: String, _ fraction: Double, estimate: UsageWindow.Estimate? = nil,
                        exhausted: Bool = false) -> UsageWindow {
        UsageWindow(id: id, kind: .weekly, scope: nil, usedFraction: fraction, windowSeconds: 604_800,
                    resetsAt: nil, estimate: estimate, isExhausted: exhausted)
    }

    private func live(_ account: AccountKey, _ windows: [UsageWindow]) -> ProviderUsage {
        ProviderUsage(account: account, windows: windows, observedAt: Date(), state: .live, plan: nil, creditBalance: nil)
    }

    private func tightest(_ readings: [AccountKey: ProviderUsage], order: [AccountKey],
                          pinned: [AccountKey: String] = [:], threshold: Double = 0.8) -> MenuBarReading? {
        MenuBarReading.tightest(
            among: order,
            usage: { readings[$0] ?? .unavailable($0, reason: .loading) },
            pinned: { pinned[$0] },
            warningAt: threshold
        )
    }

    @Test("The fullest ring wins, measured by each ring's own headline")
    func fullestWins() {
        let reading = tightest([
            claude: live(claude, [window("5h", 0.3), window("week", 0.6)]),
            codex: live(codex, [window("week", 0.45)]),
        ], order: [claude, codex])
        #expect(reading?.account == claude)
        #expect(reading?.window.id == "week")
        #expect(reading?.text(remaining: false) == "60%")
        #expect(reading?.text(remaining: true) == "40%")
        #expect(reading?.isAlert == false)
    }

    @Test("A pinned limit is what that account contributes, as on its ring")
    func pinnedLimitCounts() {
        let reading = tightest([
            claude: live(claude, [window("5h", 0.3), window("week", 0.9)]),
            codex: live(codex, [window("week", 0.5)]),
        ], order: [claude, codex], pinned: [claude: "5h"])
        #expect(reading?.account == codex)
    }

    @Test("An account with no reading, or only an inferred ring, is left out")
    func unavailableAndEstimatedAreLeftOut() {
        let reading = tightest([
            claude: .unavailable(claude, reason: .serverError),
            codex: live(codex, [window("balance", 0.95, estimate: .yourBudget)]),
            cursor: live(cursor, [window("month", 0.2)]),
        ], order: [claude, codex, cursor])
        #expect(reading?.account == cursor)
    }

    @Test("Nothing to show is nothing, not a zero")
    func nothingToShow() {
        #expect(tightest([claude: .unavailable(claude, reason: .loading)], order: [claude]) == nil)
        #expect(tightest([:], order: []) == nil)
    }

    @Test("A tie goes to the account earlier on the rail")
    func tieKeepsRailOrder() {
        let reading = tightest([
            claude: live(claude, [window("week", 0.5)]),
            codex: live(codex, [window("week", 0.5)]),
        ], order: [codex, claude])
        #expect(reading?.account == codex)
    }

    @Test("Red at the warning line, and when the provider says spent", arguments: [
        (0.79, false, false), (0.8, false, true), (0.1, true, true),
    ])
    func alert(fraction: Double, exhausted: Bool, alert: Bool) {
        let reading = tightest([claude: live(claude, [window("week", fraction, exhausted: exhausted)])], order: [claude])
        #expect(reading?.isAlert == alert)
    }
}
