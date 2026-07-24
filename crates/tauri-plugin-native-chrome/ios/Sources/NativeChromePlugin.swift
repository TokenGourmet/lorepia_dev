import SwiftRs
import Tauri
import UIKit
import WebKit
import os

private enum NativeChromeTab: String, Codable, CaseIterable {
    case home
    case library
    case create
    case account

    var title: String {
        switch self {
        case .home:
            return "홈"
        case .library:
            return "서재"
        case .create:
            return "생성"
        case .account:
            return "계정"
        }
    }

    var symbolName: String {
        switch self {
        case .home:
            return "house"
        case .library:
            return "books.vertical"
        case .create:
            return "plus.circle"
        case .account:
            return "person"
        }
    }

    var selectedSymbolName: String {
        switch self {
        case .home:
            return "house.fill"
        case .library:
            return "books.vertical.fill"
        case .create:
            return "plus.circle.fill"
        case .account:
            return "person.fill"
        }
    }

    private var fallbackSymbolName: String {
        switch self {
        case .library:
            return "book.closed"
        default:
            return symbolName
        }
    }

    private var selectedFallbackSymbolName: String {
        switch self {
        case .library:
            return "book.closed.fill"
        default:
            return selectedSymbolName
        }
    }

    func iconImage(selected: Bool) -> UIImage? {
        let configuration = UIImage.SymbolConfiguration(
            pointSize: 22,
            weight: selected ? .semibold : .medium,
            scale: .medium
        )
        let preferred = selected ? selectedSymbolName : symbolName
        let fallback =
            selected
                ? selectedFallbackSymbolName
                : fallbackSymbolName
        return (
            UIImage(
                systemName: preferred,
                withConfiguration: configuration
            )
                ?? UIImage(
                    systemName: fallback,
                    withConfiguration: configuration
                )
        )?.withRenderingMode(.alwaysTemplate)
    }
}

private enum NativeChromeAppearance: String, Decodable {
    case system
    case light
    case dark

    var interfaceStyle: UIUserInterfaceStyle {
        switch self {
        case .system:
            return .unspecified
        case .light:
            return .light
        case .dark:
            return .dark
        }
    }
}

private struct NativeChromeState: Decodable {
    let visible: Bool
    let selectedTab: NativeChromeTab
    let minimized: Bool
    let appearance: NativeChromeAppearance
    let compact: Bool

    static let initial = NativeChromeState(
        visible: false,
        selectedTab: .library,
        minimized: false,
        appearance: .system,
        compact: false
    )
}

private struct NativeChromeStatus: Encodable {
    let supported: Bool
    let active: Bool
    let compact: Bool
    let visible: Bool
    let selectedTab: NativeChromeTab
    let minimized: Bool
}

private extension Notification.Name {
    static let nativeChromeWillInstallTabShell = Notification.Name(
        "dev.lorepia.nativeChrome.willInstallTabShell"
    )
    static let prepareChromeUnderlay = Notification.Name(
        "dev.lorepia.nativeBack.prepareChromeUnderlay"
    )
    static let clearChromeUnderlay = Notification.Name(
        "dev.lorepia.nativeBack.clearChromeUnderlay"
    )
    static let nativeBackWillAcquireWebView = Notification.Name(
        "dev.lorepia.nativeBack.willAcquireWebView"
    )
    static let nativeBackDidReleaseWebView = Notification.Name(
        "dev.lorepia.nativeBack.didReleaseWebView"
    )
}

private final class SystemTabDockView: UITabBar, UITabBarDelegate {
    private var tabItems: [UITabBarItem] = []
    private var minimized = false
    private var authoritativeTab = NativeChromeTab.library
    private var pendingTab: NativeChromeTab?
    private var pendingGeneration = 0

    var onSelect: ((NativeChromeTab) -> Void)?

    init(interactive: Bool) {
        super.init(frame: .zero)

        translatesAutoresizingMaskIntoConstraints = false
        clipsToBounds = false
        isTranslucent = true
        isUserInteractionEnabled = interactive
        delegate = interactive ? self : nil
        itemPositioning = .automatic
        if !interactive {
            accessibilityElementsHidden = true
            isAccessibilityElement = false
        }

        tabItems = NativeChromeTab.allCases.enumerated().map {
            index,
            tab in
            let item = UITabBarItem(
                title: tab.title,
                image: tab.iconImage(selected: false),
                selectedImage: tab.iconImage(selected: true)
            )
            item.tag = index
            item.accessibilityLabel = tab.title
            item.accessibilityIdentifier = "native-dock-\(tab.rawValue)"
            return item
        }
        setItems(tabItems, animated: false)
        selectedItem = tabItems[
            NativeChromeTab.allCases.firstIndex(of: .library) ?? 0
        ]
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("SystemTabDockView does not support storyboards")
    }

    func apply(
        selectedTab: NativeChromeTab,
        minimized: Bool,
        appearance: NativeChromeAppearance,
        visible: Bool,
        animated _: Bool
    ) {
        overrideUserInterfaceStyle = appearance.interfaceStyle

        if self.minimized != minimized {
            self.minimized = minimized
            for (index, tab) in NativeChromeTab.allCases.enumerated() {
                tabItems[index].title = minimized ? nil : tab.title
            }
            setNeedsLayout()
            invalidateIntrinsicContentSize()
        }

        let index = NativeChromeTab.allCases.firstIndex(
            of: selectedTab
        ) ?? 0
        authoritativeTab = selectedTab
        if pendingTab == selectedTab {
            pendingTab = nil
            pendingGeneration &+= 1
        }
        if pendingTab == nil, selectedItem !== tabItems[index] {
            selectedItem = tabItems[index]
        }
        isHidden = !visible
    }

    func tabBar(
        _ tabBar: UITabBar,
        didSelect item: UITabBarItem
    ) {
        guard
            item.tag >= 0,
            item.tag < NativeChromeTab.allCases.count
        else {
            return
        }
        let tab = NativeChromeTab.allCases[item.tag]
        pendingGeneration &+= 1
        let generation = pendingGeneration
        pendingTab = tab
        onSelect?(tab)

        // A route commit normally confirms the optimistic native selection.
        // If navigation is rejected, fail back to the last committed route
        // instead of leaving native chrome and Web content out of sync.
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self] in
            guard
                let self,
                self.pendingGeneration == generation,
                self.pendingTab == tab
            else {
                return
            }
            self.pendingTab = nil
            let committedIndex =
                NativeChromeTab.allCases.firstIndex(
                    of: self.authoritativeTab
                ) ?? 0
            if self.selectedItem !== self.tabItems[committedIndex] {
                self.selectedItem = self.tabItems[committedIndex]
            }
        }
    }
}

private final class DockPlacement {
    let dock: SystemTabDockView
    private let constraints: [NSLayoutConstraint]

    init(
        dock: SystemTabDockView,
        host: UIView
    ) {
        self.dock = dock
        host.addSubview(dock)

        constraints = [
            dock.leadingAnchor.constraint(equalTo: host.leadingAnchor),
            dock.trailingAnchor.constraint(equalTo: host.trailingAnchor),
            dock.bottomAnchor.constraint(
                equalTo: host.bottomAnchor
            ),
        ]
        NSLayoutConstraint.activate(constraints)
    }

    func remove() {
        NSLayoutConstraint.deactivate(constraints)
        dock.removeFromSuperview()
    }
}

final class NativeChromePlugin: Plugin {
    private static let tabEventScripts: [NativeChromeTab: String] = [
        .home:
            "window.dispatchEvent(new CustomEvent('lorepia:native-tab',{detail:{tab:'home'}}))",
        .library:
            "window.dispatchEvent(new CustomEvent('lorepia:native-tab',{detail:{tab:'library'}}))",
        .create:
            "window.dispatchEvent(new CustomEvent('lorepia:native-tab',{detail:{tab:'create'}}))",
        .account:
            "window.dispatchEvent(new CustomEvent('lorepia:native-tab',{detail:{tab:'account'}}))",
    ]

    private let logger = Logger(
        subsystem: "dev.lorepia.client",
        category: "NativeChrome"
    )
    private weak var webview: WKWebView?
    private weak var rootViewController: UIViewController?
    private var state = NativeChromeState.initial
    private var livePlacement: DockPlacement?
    private var underlayPlacement: DockPlacement?
    private var observerTokens: [NSObjectProtocol] = []
    private var underlayGeneration = 0
    private var underlayActive = false

    @objc override public func load(webview: WKWebView) {
        self.webview = webview
        rootViewController = manager.viewController
        registerUnderlayObservers()
        logger.notice("Native chrome plugin loaded")
    }

    @objc public func setState(_ invoke: Invoke) throws {
        let requestedState = try invoke.parseArgs(
            NativeChromeState.self
        )
        resolveOnMain(invoke) {
            guard self.isSupported else {
                return self.unsupportedStatus
            }
            self.state = requestedState
            self.applyLiveState()
            return self.currentStatus
        }
    }

    @objc public func status(_ invoke: Invoke) {
        resolveOnMain(invoke) {
            self.currentStatus
        }
    }

    deinit {
        for token in observerTokens {
            NotificationCenter.default.removeObserver(token)
        }
        livePlacement?.remove()
    }

    private var isSupported: Bool {
        if #available(iOS 26.0, *) {
            return true
        }
        return false
    }

    private var unsupportedStatus: NativeChromeStatus {
        NativeChromeStatus(
            supported: false,
            active: false,
            compact: false,
            visible: false,
            selectedTab: .library,
            minimized: false
        )
    }

    private var currentStatus: NativeChromeStatus {
        guard isSupported else {
            return unsupportedStatus
        }
        let active = state.compact && livePlacement != nil
        let visible = active && state.visible
        return NativeChromeStatus(
            supported: true,
            active: active,
            compact: state.compact,
            visible: visible,
            selectedTab: state.selectedTab,
            minimized: false
        )
    }

    private func resolveOnMain(
        _ invoke: Invoke,
        operation: @escaping () -> NativeChromeStatus
    ) {
        DispatchQueue.main.async {
            invoke.resolve(operation())
        }
    }

    private func applyLiveState() {
        guard isSupported else {
            livePlacement?.dock.apply(
                selectedTab: .library,
                minimized: false,
                appearance: .system,
                visible: false,
                animated: false
            )
            return
        }

        if state.compact {
            installLiveDockIfNeeded()
        }
        guard
            let livePlacement,
            rootViewController?.view != nil
                || manager.viewController?.view != nil
        else {
            return
        }

        let status = currentStatus
        livePlacement.dock.apply(
            selectedTab: state.selectedTab,
            minimized: false,
            appearance: state.appearance,
            visible: status.visible && !underlayActive,
            animated: false
        )
        if status.visible && !underlayActive {
            rootViewController?.view.bringSubviewToFront(
                livePlacement.dock
            )
        }
    }

    private func installLiveDockIfNeeded() {
        guard livePlacement == nil else {
            return
        }
        guard
            let rootViewController =
                rootViewController ?? manager.viewController
        else {
            logger.error("Unable to install native dock without a root view controller")
            return
        }

        self.rootViewController = rootViewController
        NotificationCenter.default.post(
            name: .nativeChromeWillInstallTabShell,
            object: rootViewController
        )
        let dock = SystemTabDockView(interactive: true)
        dock.onSelect = { [weak self] tab in
            self?.select(tab)
        }
        let placement = DockPlacement(
            dock: dock,
            host: rootViewController.view
        )
        livePlacement = placement
        dock.apply(
            selectedTab: state.selectedTab,
            minimized: false,
            appearance: state.appearance,
            visible: false,
            animated: false
        )
        logger.notice("Stable native four-item dock installed")
    }

    private func select(_ tab: NativeChromeTab) {
        let status = currentStatus
        guard status.active && status.visible else {
            return
        }

        guard let script = Self.tabEventScripts[tab] else {
            return
        }
        webview?.evaluateJavaScript(script) { [weak self] _, error in
            if let error {
                self?.logger.error(
                    "Unable to deliver native tab selection: \(error.localizedDescription, privacy: .public)"
                )
            }
        }
    }

    private func registerUnderlayObservers() {
        guard observerTokens.isEmpty else {
            return
        }
        let center = NotificationCenter.default
        observerTokens = [
            center.addObserver(
                forName: .prepareChromeUnderlay,
                object: nil,
                queue: .main
            ) { [weak self] notification in
                self?.prepareUnderlay(notification)
            },
            center.addObserver(
                forName: .clearChromeUnderlay,
                object: nil,
                queue: .main
            ) { [weak self] _ in
                self?.clearUnderlay()
            },
            center.addObserver(
                forName: .nativeBackWillAcquireWebView,
                object: nil,
                queue: .main
            ) { [weak self] notification in
                self?.beginNativeBackWebViewLease(notification)
            },
            center.addObserver(
                forName: .nativeBackDidReleaseWebView,
                object: nil,
                queue: .main
            ) { [weak self] notification in
                self?.endNativeBackWebViewLease(notification)
            },
        ]
    }

    private func beginNativeBackWebViewLease(
        _ notification: Notification
    ) {
        guard
            let leasedWebView = notification.object as? WKWebView,
            leasedWebView === webview
        else {
            return
        }
        logger.notice("Native back acquired the shared WebView lease")
    }

    private func endNativeBackWebViewLease(
        _ notification: Notification
    ) {
        guard
            let releasedWebView = notification.object as? WKWebView,
            releasedWebView === webview
        else {
            return
        }
        applyLiveState()
        logger.notice("Native back released the shared WebView lease")
    }

    private func prepareUnderlay(_ notification: Notification) {
        guard let host = notification.object as? UIView else {
            logger.error("Native back underlay notification did not provide a UIView host")
            return
        }

        underlayGeneration += 1
        let generation = underlayGeneration
        underlayPlacement?.remove()
        underlayPlacement = nil

        let sourceState = state
        let sourceStatus = currentStatus
        guard sourceStatus.active && sourceStatus.visible else {
            return
        }
        underlayActive = true
        livePlacement?.dock.apply(
            selectedTab: sourceState.selectedTab,
            minimized: false,
            appearance: sourceState.appearance,
            visible: false,
            animated: false
        )

        // NativeBack may post while it is assembling the source snapshot.
        // Defer one main-loop turn so this live glass stays above that raster
        // surface instead of being flattened into or covered by it.
        DispatchQueue.main.async { [weak self, weak host] in
            guard let self, self.underlayGeneration == generation else {
                return
            }
            guard let host else {
                self.underlayActive = false
                self.applyLiveState()
                return
            }

            let dock = SystemTabDockView(interactive: false)
            let placement = DockPlacement(
                dock: dock,
                host: host
            )
            dock.apply(
                selectedTab: sourceState.selectedTab,
                minimized: false,
                appearance: sourceState.appearance,
                visible: true,
                animated: false
            )
            host.bringSubviewToFront(dock)
            self.underlayPlacement = placement
            self.logger.notice("Native dock underlay prepared")
        }
    }

    private func clearUnderlay() {
        underlayGeneration += 1
        underlayPlacement?.remove()
        underlayPlacement = nil
        underlayActive = false
        applyLiveState()
        logger.notice("Native dock underlay cleared")
    }
}

@_cdecl("init_plugin_native_chrome")
func initPlugin() -> Plugin {
    NativeChromePlugin()
}
