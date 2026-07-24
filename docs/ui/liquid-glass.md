# LorePia Liquid Glass

This implementation is designed for the existing Svelte 5 UI inside the Tauri 2 WebView shell.

## Rendering layers

1. `backdrop-filter` provides the platform WebView blur, saturation, and contrast pass.
2. Theme tokens provide translucent tint, edge shade, specular light, and fallback fills.
3. A transparent canvas draws the pointer-following light and expanding press wave without adding inline styles, preserving the app's strict `style-src 'self'` CSP.
4. CSS transforms provide the short press compression and spring-like release/materialization motion.

The canvas redraws only while a pointer/focus transition or ripple is active. Its backing buffer is capped at device pixel ratio 2 to avoid excessive memory and fill-rate use on high-density Android displays.

## Scope

The first integration is the chat composer. The shared component can later be used for compact toolbars, model selectors, and floating status panels. Avoid applying it to every chat bubble; each backdrop surface creates an additional compositing layer.

## Platform fallback

- Modern Android System WebView / WKWebView / desktop WebView: blur, tint, canvas light, ripple, and spring motion.
- WebViews without backdrop-filter: opaque themed fallback fill plus the same interaction light and motion.
- Reduced motion: no materialization or press transform; the visual surface remains available.
