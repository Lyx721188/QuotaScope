import AppKit

/// One account's row in the menu bar item's menu: mark, name and figure on
/// top, the limit and its reset beneath.
///
/// **A view, not an attributed title.** A title is laid out left of the
/// menu's key-equivalent column, so a figure right-aligned inside it stopped
/// short of the column's edge while "⌘," and "⌘Q" ran to it — two right edges
/// a few points apart, which is exactly what reads as misaligned. A view spans
/// the whole row, so the figure is pinned to the edge the shortcuts end at.
///
/// A view in a menu gets none of a menu item's behaviour for free: it draws
/// its own highlight, turns its text white on it, and sends its item's action
/// on a click.
final class MenuUsageRowView: NSView {
    /// Measured against the system's own items on macOS 26: `trailing` puts
    /// the figure's last glyph on the edge "⌘Q" ends at (14 left it a point
    /// past). The highlight is inset by the same margin the system's is.
    private enum Metrics {
        static let highlightInset: CGFloat = 5
        static let highlightRadius: CGFloat = 6
        static let leading: CGFloat = 14
        static let trailing: CGFloat = 15.5
        static let iconSize: CGFloat = 16
        static let iconGap: CGFloat = 6
        static let height: CGFloat = 38
    }

    private let icon = NSImageView()
    private let name = NSTextField(labelWithString: "")
    private let figure = NSTextField(labelWithString: "")
    private let detail = NSTextField(labelWithString: "")
    private let isAlert: Bool

    init(mark: NSImage?, name: String, figure: String, detail: String?, alert: Bool) {
        isAlert = alert
        super.init(frame: NSRect(x: 0, y: 0, width: 280, height: Metrics.height))
        autoresizingMask = [.width]

        icon.image = mark
        icon.imageScaling = .scaleProportionallyUpOrDown
        self.name.stringValue = name
        self.name.font = .menuFont(ofSize: 0)
        self.name.lineBreakMode = .byTruncatingTail
        self.figure.stringValue = figure
        self.figure.font = .monospacedDigitSystemFont(ofSize: NSFont.menuFont(ofSize: 0).pointSize, weight: .medium)
        self.figure.alignment = .right
        self.detail.stringValue = detail ?? ""
        self.detail.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        self.detail.lineBreakMode = .byTruncatingTail

        for view in [icon, self.name, self.figure, self.detail] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        self.figure.setContentCompressionResistancePriority(.required, for: .horizontal)
        self.figure.setContentHuggingPriority(.required, for: .horizontal)
        self.name.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        let textLeading = Metrics.leading + Metrics.iconSize + Metrics.iconGap
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Metrics.leading),
            icon.widthAnchor.constraint(equalToConstant: Metrics.iconSize),
            icon.heightAnchor.constraint(equalToConstant: Metrics.iconSize),
            icon.centerYAnchor.constraint(equalTo: self.name.centerYAnchor),

            self.name.leadingAnchor.constraint(equalTo: leadingAnchor, constant: textLeading),
            self.name.topAnchor.constraint(equalTo: topAnchor, constant: 3),
            self.name.trailingAnchor.constraint(lessThanOrEqualTo: self.figure.leadingAnchor, constant: -16),

            self.figure.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -Metrics.trailing),
            self.figure.firstBaselineAnchor.constraint(equalTo: self.name.firstBaselineAnchor),

            self.detail.leadingAnchor.constraint(equalTo: self.name.leadingAnchor),
            self.detail.topAnchor.constraint(equalTo: self.name.bottomAnchor, constant: 0),
            self.detail.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -Metrics.trailing),

            heightAnchor.constraint(equalToConstant: Metrics.height),
            // Wide enough for its own text; the menu stretches it to the rest.
            widthAnchor.constraint(greaterThanOrEqualToConstant: 240),
        ])
        applyColours()
    }

    required init?(coder: NSCoder) { nil }

    private var isHighlighted: Bool { enclosingMenuItem?.isHighlighted ?? false }

    /// White on the highlight, as every menu item's text is; otherwise the
    /// ordinary label colours, and red for a figure past the warning line.
    private func applyColours() {
        let lit = isHighlighted
        name.textColor = lit ? .selectedMenuItemTextColor : .labelColor
        detail.textColor = lit ? .selectedMenuItemTextColor : .secondaryLabelColor
        figure.textColor = lit ? .selectedMenuItemTextColor : (isAlert ? .systemRed : .labelColor)
        icon.contentTintColor = lit ? .selectedMenuItemTextColor : .labelColor
    }

    override func viewWillDraw() {
        applyColours()
        super.viewWillDraw()
    }

    override func draw(_ dirtyRect: NSRect) {
        guard isHighlighted else { return }
        NSColor.selectedContentBackgroundColor.setFill()
        NSBezierPath(
            roundedRect: bounds.insetBy(dx: Metrics.highlightInset, dy: 0),
            xRadius: Metrics.highlightRadius,
            yRadius: Metrics.highlightRadius
        ).fill()
    }

    override func mouseUp(with event: NSEvent) {
        guard let item = enclosingMenuItem, let menu = item.menu else { return }
        menu.cancelTracking()
        if let action = item.action {
            NSApp.sendAction(action, to: item.target, from: item)
        }
    }
}
