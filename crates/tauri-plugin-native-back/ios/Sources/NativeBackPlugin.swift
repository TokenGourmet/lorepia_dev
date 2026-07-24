import SwiftRs
import Tauri
import UIKit
import WebKit
import os

private struct NativeBackStatus: Encodable {
    let supported: Bool
    let active: Bool
    let gestureEnabled: Bool
}

private final class SetEnabledArgs: Decodable {
    let enabled: Bool
}

final class NativeBackPlugin: Plugin, UINavigationControllerDelegate {
    private static let nativeChromeWillInstallTabShellNotification =
        Notification.Name(
            "dev.lorepia.nativeChrome.willInstallTabShell"
        )
    private static let prepareChromeUnderlayNotification =
        Notification.Name("dev.lorepia.nativeBack.prepareChromeUnderlay")
    private static let clearChromeUnderlayNotification =
        Notification.Name("dev.lorepia.nativeBack.clearChromeUnderlay")
    private static let willAcquireWebViewNotification =
        Notification.Name("dev.lorepia.nativeBack.willAcquireWebView")
    private static let didReleaseWebViewNotification =
        Notification.Name("dev.lorepia.nativeBack.didReleaseWebView")
    private static let commitScript =
        "window.dispatchEvent(new Event('lorepia:native-back'))"
    private static let roomInfoScript =
        "window.dispatchEvent(new Event('lorepia:native-room-info'))"
    private let logger = Logger(
        subsystem: "dev.lorepia.client",
        category: "NativeBack"
    )
    private weak var webview: WKWebView?
    private weak var rootViewController: UIViewController?
    private var navigationController: UINavigationController?
    private var ownsNavigationController = false
    private var sourceController: UIViewController?
    private var destinationController: UIViewController?
    private var sourceSnapshot: UIView?
    private var active = false
    private var gestureEnabled = false
    private var sourceTabBarVisible = false
    private var nativeChromeObserver: NSObjectProtocol?
    private var snapshotGeneration = 0
    private var webViewLeaseActive = false
    private var pendingSnapshotCompletions: [
        (Result<Void, Error>) -> Void
    ] = []

    @objc override public func load(webview: WKWebView) {
        self.webview = webview
        self.rootViewController = manager.viewController
        webview.allowsBackForwardNavigationGestures = false
        registerNativeChromeObserver()
        logger.notice("Native back plugin loaded")
    }

    deinit {
        if let nativeChromeObserver {
            NotificationCenter.default.removeObserver(
                nativeChromeObserver
            )
        }
    }

    @objc public func complete(_ invoke: Invoke) {
        resolveOnMain(invoke) {
            self.completeTransition()
            return self.currentStatus()
        }
    }

    @objc public func pop(_ invoke: Invoke) {
        resolveOnMain(invoke) {
            guard self.active, let navigationController = self.navigationController else {
                return self.currentStatus()
            }
            self.gestureEnabled = false
            navigationController.popViewController(animated: true)
            return self.currentStatus()
        }
    }

    @objc public func prepare(_ invoke: Invoke) {
        DispatchQueue.main.async {
            guard self.isSystemGestureSupported else {
                invoke.resolve(self.currentStatus())
                return
            }

            self.prepareTransition { result in
                switch result {
                case .success:
                    invoke.resolve(self.currentStatus())
                case let .failure(error):
                    self.rejectCoordination(invoke, error: error)
                }
            }
        }
    }

    @objc public func setEnabled(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(SetEnabledArgs.self)
        resolveOnMain(invoke) {
            self.setSystemGestureEnabled(args.enabled && self.active)
            return self.currentStatus()
        }
    }

    @objc public func status(_ invoke: Invoke) {
        resolveOnMain(invoke) {
            self.currentStatus()
        }
    }

    func navigationController(
        _ navigationController: UINavigationController,
        didShow viewController: UIViewController,
        animated: Bool
    ) {
        guard
            active,
            let sourceController,
            viewController === sourceController
        else {
            return
        }

        active = false
        gestureEnabled = false
        resetAdditionalSafeAreaInsets(
            sourceController,
            destinationController
        )
        navigationController.setNavigationBarHidden(true, animated: false)
        attachWebView(to: sourceController, beneathSnapshot: true)
        destinationController = nil
        releaseWebViewLease()

        webview?.evaluateJavaScript(Self.commitScript) { [weak self] _, error in
            if let error {
                self?.logger.error(
                    "Unable to deliver native back commit: \(error.localizedDescription, privacy: .public)"
                )
            }
        }
        logger.notice("Native interactive pop committed")
    }

    private var isSystemGestureSupported: Bool {
        if #available(iOS 26.0, *) {
            return true
        }
        return false
    }

    private func currentStatus() -> NativeBackStatus {
        NativeBackStatus(
            supported: isSystemGestureSupported,
            active: active,
            gestureEnabled: gestureEnabled
        )
    }

    private func resolveOnMain(
        _ invoke: Invoke,
        operation: @escaping () throws -> NativeBackStatus
    ) {
        DispatchQueue.main.async {
            do {
                invoke.resolve(try operation())
            } catch {
                self.rejectCoordination(invoke, error: error)
            }
        }
    }

    private func rejectCoordination(
        _ invoke: Invoke,
        error: Error
    ) {
        invoke.reject(
            "Unable to coordinate native back navigation",
            code: "NATIVE_BACK_COORDINATION_FAILED",
            error: error
        )
    }

    private func installNavigationHostIfNeeded() throws {
        guard let webview else {
            throw NativeBackError.webViewUnavailable
        }
        guard let rootViewController = rootViewController ?? manager.viewController else {
            throw NativeBackError.rootViewControllerUnavailable
        }

        if active, navigationController != nil {
            return
        }

        if
            let sharedNavigation =
                selectedTabNavigationController(
                    in: rootViewController
                ),
            let sharedSource =
                sharedNavigation.viewControllers.first
        {
            adoptSharedNavigationHost(
                sharedNavigation,
                sourceController: sharedSource,
                webview: webview
            )
            return
        }

        if navigationController != nil {
            return
        }

        let sourceController = UIViewController()
        configureSourceController(sourceController)

        let navigationController = UINavigationController(
            rootViewController: sourceController
        )
        navigationController.delegate = self
        configureNavigationBar(navigationController)
        navigationController.setNavigationBarHidden(true, animated: false)
        navigationController.view.backgroundColor = .systemBackground

        rootViewController.addChild(navigationController)
        rootViewController.view.addSubview(navigationController.view)
        navigationController.view.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            navigationController.view.leadingAnchor.constraint(
                equalTo: rootViewController.view.leadingAnchor
            ),
            navigationController.view.trailingAnchor.constraint(
                equalTo: rootViewController.view.trailingAnchor
            ),
            navigationController.view.topAnchor.constraint(
                equalTo: rootViewController.view.topAnchor
            ),
            navigationController.view.bottomAnchor.constraint(
                equalTo: rootViewController.view.bottomAnchor
            ),
        ])
        navigationController.didMove(toParent: rootViewController)

        self.rootViewController = rootViewController
        self.sourceController = sourceController
        self.navigationController = navigationController
        ownsNavigationController = true
        attachWebView(to: sourceController, beneathSnapshot: false)
        webview.allowsBackForwardNavigationGestures = false
        configureGestureCompetition(
            webview: webview,
            navigationController: navigationController
        )
    }

    private func selectedTabNavigationController(
        in rootViewController: UIViewController
    ) -> UINavigationController? {
        for child in rootViewController.children {
            guard let tabs = child as? UITabBarController else {
                continue
            }
            guard let selected = tabs.selectedViewController else {
                continue
            }
            if let navigation = descendantNavigationController(
                in: selected
            ) {
                return navigation
            }
        }
        return nil
    }

    private func descendantNavigationController(
        in controller: UIViewController
    ) -> UINavigationController? {
        if let navigation = controller as? UINavigationController {
            return navigation
        }
        for child in controller.children {
            if let navigation = descendantNavigationController(in: child) {
                return navigation
            }
        }
        return nil
    }

    private func adoptSharedNavigationHost(
        _ navigationController: UINavigationController,
        sourceController: UIViewController,
        webview: WKWebView
    ) {
        if self.navigationController !== navigationController {
            if ownsNavigationController {
                releaseOwnedNavigationHost()
            } else {
                self.navigationController?.delegate = nil
            }
        }

        configureSourceController(sourceController)
        navigationController.delegate = self
        configureNavigationBar(navigationController)
        navigationController.setViewControllers(
            [sourceController],
            animated: false
        )
        navigationController.setNavigationBarHidden(
            true,
            animated: false
        )

        self.navigationController = navigationController
        self.sourceController = sourceController
        ownsNavigationController = false
        attachWebView(to: sourceController, beneathSnapshot: false)
        webview.allowsBackForwardNavigationGestures = false
        configureGestureCompetition(
            webview: webview,
            navigationController: navigationController
        )
        logger.notice("Native back adopted the selected tab navigation host")
    }

    private func configureSourceController(
        _ sourceController: UIViewController
    ) {
        sourceController.view.backgroundColor = .systemBackground
        let backItem = UIBarButtonItem(
            title: "",
            style: .plain,
            target: nil,
            action: nil
        )
        backItem.accessibilityLabel = "이전 화면으로 돌아가기"
        if #available(iOS 26.0, *) {
            backItem.hidesSharedBackground = true
        }
        sourceController.navigationItem.backBarButtonItem = backItem
        sourceController.navigationItem.backButtonDisplayMode = .minimal
    }

    private func registerNativeChromeObserver() {
        guard nativeChromeObserver == nil else {
            return
        }
        nativeChromeObserver = NotificationCenter.default.addObserver(
            forName: Self.nativeChromeWillInstallTabShellNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.releaseOwnedNavigationHost()
        }
    }

    private func releaseOwnedNavigationHost() {
        guard
            ownsNavigationController,
            !active,
            let navigationController,
            let rootViewController =
                rootViewController ?? manager.viewController
        else {
            return
        }

        cancelSnapshotPreparation()
        let releasedWebView = webview
        releasedWebView?.removeFromSuperview()
        navigationController.delegate = nil
        navigationController.willMove(toParent: nil)
        navigationController.view.removeFromSuperview()
        navigationController.removeFromParent()

        if let releasedWebView {
            releasedWebView.translatesAutoresizingMaskIntoConstraints = false
            rootViewController.view.addSubview(releasedWebView)
            NSLayoutConstraint.activate([
                releasedWebView.leadingAnchor.constraint(
                    equalTo: rootViewController.view.leadingAnchor
                ),
                releasedWebView.trailingAnchor.constraint(
                    equalTo: rootViewController.view.trailingAnchor
                ),
                releasedWebView.topAnchor.constraint(
                    equalTo: rootViewController.view.topAnchor
                ),
                releasedWebView.bottomAnchor.constraint(
                    equalTo: rootViewController.view.bottomAnchor
                ),
            ])
        }

        self.navigationController = nil
        sourceController = nil
        destinationController = nil
        sourceSnapshot?.removeFromSuperview()
        sourceSnapshot = nil
        ownsNavigationController = false
        gestureEnabled = false
        logger.notice("Native back released its fallback navigation host")
    }

    private func configureNavigationBar(
        _ navigationController: UINavigationController
    ) {
        let appearance = UINavigationBarAppearance()
        appearance.configureWithTransparentBackground()
        appearance.backgroundColor = .clear
        appearance.backgroundEffect = nil
        appearance.shadowColor = .clear

        let navigationBar = navigationController.navigationBar
        navigationBar.standardAppearance = appearance
        navigationBar.scrollEdgeAppearance = appearance
        navigationBar.compactAppearance = appearance
        if #available(iOS 15.0, *) {
            navigationBar.compactScrollEdgeAppearance = appearance
        }
        navigationBar.isTranslucent = true
        navigationBar.prefersLargeTitles = false

        // Keep a real, interactive navigation bar so UIKit can own iOS 26's
        // content-pop recognizer. Mask only its pixels: hiding the view or its
        // generated back control also makes the system recognizer ineligible.
        // Hit testing and accessibility stay intact above the visible,
        // chat-owned header.
        navigationBar.tintColor = .clear
        navigationBar.isUserInteractionEnabled = true
        concealNavigationBarChrome(navigationBar)
    }

    private func concealNavigationBarChrome(
        _ navigationBar: UINavigationBar
    ) {
        let emptyChromeMask = CAShapeLayer()
        emptyChromeMask.frame = navigationBar.bounds
        emptyChromeMask.path = UIBezierPath(rect: .zero).cgPath
        emptyChromeMask.fillColor = UIColor.black.cgColor
        navigationBar.layer.mask = emptyChromeMask
    }

    private func compensateForVisibleNavigationBar(
        _ navigationController: UINavigationController,
        controllers: UIViewController...
    ) {
        navigationController.view.layoutIfNeeded()
        let navigationBar = navigationController.navigationBar
        concealNavigationBarChrome(navigationBar)
        let navigationBarHeight = navigationBar.frame.height
        for controller in controllers {
            var insets = controller.additionalSafeAreaInsets
            insets.top = -navigationBarHeight
            controller.additionalSafeAreaInsets = insets
        }
    }

    private func makeRoomTitleHitTarget() -> UIView {
        let target = UIControl()
        target.backgroundColor = .clear
        target.isAccessibilityElement = true
        target.accessibilityLabel = "대화 설정 열기"
        target.accessibilityTraits = .button
        target.addTarget(
            self,
            action: #selector(openRoomInfo),
            for: .touchUpInside
        )
        NSLayoutConstraint.activate([
            target.widthAnchor.constraint(equalToConstant: 88),
            target.heightAnchor.constraint(equalToConstant: 44),
        ])
        return target
    }

    @objc private func openRoomInfo() {
        guard active else {
            return
        }
        webview?.evaluateJavaScript(Self.roomInfoScript) {
            [weak self] _, error in
            if let error {
                self?.logger.error(
                    "Unable to open native room info: \(error.localizedDescription, privacy: .public)"
                )
            }
        }
    }

    private func resetAdditionalSafeAreaInsets(
        _ controllers: UIViewController?...
    ) {
        for controller in controllers.compactMap({ $0 }) {
            var insets = controller.additionalSafeAreaInsets
            insets.top = 0
            controller.additionalSafeAreaInsets = insets
        }
    }

    private func configureGestureCompetition(
        webview: WKWebView,
        navigationController: UINavigationController
    ) {
        guard
            #available(iOS 26.0, *),
            let contentPopGestureRecognizer =
                navigationController.interactiveContentPopGestureRecognizer
        else {
            return
        }

        // WKWebView's scroll pan covers the entire content area. Give UIKit's
        // system back swipe the first chance to recognize a horizontal pop;
        // vertical pans fall through to normal WebView scrolling as soon as
        // the content-pop recognizer fails.
        webview.scrollView.panGestureRecognizer.require(
            toFail: contentPopGestureRecognizer
        )
        logger.notice("Native content pop gesture priority configured")
    }

    private func prepareTransition(
        completion: @escaping (Result<Void, Error>) -> Void
    ) {
        do {
            try installNavigationHostIfNeeded()
        } catch {
            completion(.failure(error))
            return
        }

        guard !active else {
            completion(.success(()))
            return
        }
        guard
            let webview,
            let sourceController
        else {
            completion(.failure(NativeBackError.navigationHostUnavailable))
            return
        }

        if !pendingSnapshotCompletions.isEmpty {
            pendingSnapshotCompletions.append(completion)
            return
        }
        snapshotGeneration &+= 1
        let generation = snapshotGeneration
        pendingSnapshotCompletions.append(completion)

        sourceController.view.layoutIfNeeded()
        webview.layoutIfNeeded()
        if #available(iOS 18.0, *) {
            sourceTabBarVisible =
                !(navigationController?.tabBarController?
                    .isTabBarHidden ?? true)
        } else {
            sourceTabBarVisible = false
        }

        let configuration = WKSnapshotConfiguration()
        configuration.rect = webview.bounds
        configuration.afterScreenUpdates = true
        webview.takeSnapshot(with: configuration) {
            [weak self, weak webview, weak sourceController] image, error in
            DispatchQueue.main.async {
                guard let self else {
                    return
                }
                guard self.snapshotGeneration == generation else {
                    return
                }
                guard
                    let webview,
                    let sourceController,
                    self.webview === webview,
                    self.sourceController === sourceController
                else {
                    self.finishSnapshotPreparation(
                        generation: generation,
                        result: .failure(
                            NativeBackError.navigationHostUnavailable
                        )
                    )
                    return
                }

                if let error {
                    self.logger.error(
                        "WKWebView snapshot failed; using hierarchy fallback: \(error.localizedDescription, privacy: .public)"
                    )
                }

                guard
                    let snapshotImage =
                        image ?? self.drawHierarchySnapshot(of: webview)
                else {
                    self.finishSnapshotPreparation(
                        generation: generation,
                        result: .failure(
                            NativeBackError.snapshotCaptureFailed
                        )
                    )
                    return
                }

                self.finishPreparingTransition(
                    snapshotImage: snapshotImage,
                    webview: webview,
                    sourceController: sourceController
                )
                self.finishSnapshotPreparation(
                    generation: generation,
                    result: .success(())
                )
            }
        }
    }

    private func finishSnapshotPreparation(
        generation: Int,
        result: Result<Void, Error>
    ) {
        guard snapshotGeneration == generation else {
            return
        }
        let completions = pendingSnapshotCompletions
        pendingSnapshotCompletions.removeAll()
        for completion in completions {
            completion(result)
        }
    }

    private func cancelSnapshotPreparation() {
        guard !pendingSnapshotCompletions.isEmpty else {
            return
        }
        snapshotGeneration &+= 1
        let completions = pendingSnapshotCompletions
        pendingSnapshotCompletions.removeAll()
        for completion in completions {
            completion(
                .failure(NativeBackError.snapshotPreparationCancelled)
            )
        }
    }

    private func drawHierarchySnapshot(of webview: WKWebView) -> UIImage? {
        guard
            webview.bounds.width > 0,
            webview.bounds.height > 0
        else {
            return nil
        }

        var didDrawHierarchy = false
        let renderer = UIGraphicsImageRenderer(bounds: webview.bounds)
        let image = renderer.image { _ in
            didDrawHierarchy = webview.drawHierarchy(
                in: webview.bounds,
                afterScreenUpdates: true
            )
        }
        return didDrawHierarchy ? image : nil
    }

    private func finishPreparingTransition(
        snapshotImage: UIImage,
        webview: WKWebView,
        sourceController: UIViewController
    ) {
        guard !active else {
            return
        }

        sourceSnapshot?.removeFromSuperview()
        acquireWebViewLease()
        NotificationCenter.default.post(
            name: Self.prepareChromeUnderlayNotification,
            object: sourceController.view
        )
        let snapshot = UIImageView(image: snapshotImage)
        snapshot.frame = sourceController.view.bounds
        snapshot.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        snapshot.contentMode = .scaleToFill
        snapshot.clipsToBounds = true
        snapshot.isUserInteractionEnabled = false
        snapshot.isAccessibilityElement = false
        // Keep the native dock underlay live above the WebView snapshot. The
        // destination controller covers both until UIKit begins its
        // interactive pop, at which point the previous page and its dock move
        // together instead of the dock popping in after navigation commits.
        sourceController.view.insertSubview(snapshot, at: 0)
        sourceSnapshot = snapshot

        let destinationController = UIViewController()
        destinationController.view.backgroundColor = .systemBackground
        destinationController.hidesBottomBarWhenPushed = true
        destinationController.navigationItem.titleView =
            makeRoomTitleHitTarget()
        attachWebView(to: destinationController, beneathSnapshot: false)

        self.destinationController = destinationController
        active = true
        setSystemGestureEnabled(false)
        logger.notice("Native navigation snapshot prepared")
    }

    private func completeTransition() {
        cancelSnapshotPreparation()
        guard
            active
                || destinationController != nil
                || sourceSnapshot != nil
        else {
            return
        }

        gestureEnabled = false

        if
            active,
            let navigationController,
            let sourceController
        {
            active = false
            navigationController.setViewControllers(
                [sourceController],
                animated: false
            )
            attachWebView(to: sourceController, beneathSnapshot: true)
        }

        resetAdditionalSafeAreaInsets(
            sourceController,
            destinationController
        )
        navigationController?.setNavigationBarHidden(
            true,
            animated: false
        )
        destinationController = nil
        sourceSnapshot?.removeFromSuperview()
        sourceSnapshot = nil
        sourceTabBarVisible = false
        NotificationCenter.default.post(
            name: Self.clearChromeUnderlayNotification,
            object: nil
        )
        releaseWebViewLease()
        logger.notice("Native navigation transition completed")
    }

    private func acquireWebViewLease() {
        guard !webViewLeaseActive, let webview else {
            return
        }
        webViewLeaseActive = true
        NotificationCenter.default.post(
            name: Self.willAcquireWebViewNotification,
            object: webview
        )
    }

    private func releaseWebViewLease() {
        guard webViewLeaseActive, let webview else {
            return
        }
        webViewLeaseActive = false
        NotificationCenter.default.post(
            name: Self.didReleaseWebViewNotification,
            object: webview
        )
    }

    private func attachWebView(
        to controller: UIViewController,
        beneathSnapshot: Bool
    ) {
        guard let webview else {
            return
        }

        webview.removeFromSuperview()
        webview.translatesAutoresizingMaskIntoConstraints = false
        if beneathSnapshot, let sourceSnapshot {
            controller.view.insertSubview(webview, belowSubview: sourceSnapshot)
        } else {
            controller.view.addSubview(webview)
        }
        NSLayoutConstraint.activate([
            webview.leadingAnchor.constraint(
                equalTo: controller.view.leadingAnchor
            ),
            webview.trailingAnchor.constraint(
                equalTo: controller.view.trailingAnchor
            ),
            webview.topAnchor.constraint(
                equalTo: controller.view.topAnchor
            ),
            webview.bottomAnchor.constraint(
                equalTo: controller.view.bottomAnchor
            ),
        ])
    }

    private func setSystemGestureEnabled(_ enabled: Bool) {
        // UIKit owns interactiveContentPopGestureRecognizer completely. Its
        // public property is only for failure requirements, so arming is done
        // by making the real navigation stack eligible instead of mutating or
        // targeting the recognizer.
        guard
            active,
            isSystemGestureSupported,
            let webview,
            let navigationController,
            let sourceController,
            let destinationController
        else {
            gestureEnabled = false
            return
        }

        if enabled {
            if #available(iOS 18.0, *), sourceTabBarVisible {
                navigationController.tabBarController?
                    .setTabBarHidden(false, animated: false)
            }
            navigationController.setNavigationBarHidden(
                false,
                animated: false
            )
            navigationController.setViewControllers(
                [sourceController, destinationController],
                animated: false
            )
            compensateForVisibleNavigationBar(
                navigationController,
                controllers: sourceController,
                destinationController
            )
            configureGestureCompetition(
                webview: webview,
                navigationController: navigationController
            )
            if #available(iOS 26.0, *) {
                gestureEnabled =
                    navigationController
                        .interactiveContentPopGestureRecognizer != nil
            } else {
                gestureEnabled = false
            }
        } else {
            gestureEnabled = false
            resetAdditionalSafeAreaInsets(
                sourceController,
                destinationController
            )
            navigationController.setViewControllers(
                [destinationController],
                animated: false
            )
            navigationController.setNavigationBarHidden(
                true,
                animated: false
            )
        }
        logger.notice(
            "Native content pop armed: \(self.gestureEnabled, privacy: .public)"
        )
    }
}

private enum NativeBackError: LocalizedError {
    case navigationHostUnavailable
    case rootViewControllerUnavailable
    case snapshotCaptureFailed
    case snapshotPreparationCancelled
    case webViewUnavailable

    var errorDescription: String? {
        switch self {
        case .navigationHostUnavailable:
            return "The native navigation host is unavailable."
        case .rootViewControllerUnavailable:
            return "The Tauri root view controller is unavailable."
        case .snapshotCaptureFailed:
            return "The previous WebView surface could not be captured."
        case .snapshotPreparationCancelled:
            return "The previous-page snapshot preparation was cancelled."
        case .webViewUnavailable:
            return "The Tauri WKWebView is unavailable."
        }
    }
}

@_cdecl("init_plugin_native_back")
func initPlugin() -> Plugin {
    NativeBackPlugin()
}
