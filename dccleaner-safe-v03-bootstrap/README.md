# DC Cleaner Safe 0.3 CI bootstrap

This directory transports the reviewed DC Cleaner Safe 0.3 source archive into an isolated GitHub Actions Android build.

## Pinned source

- Exact source archive: `DC-Cleaner-Safe-0.3.0-source.tar.gz`
- Source archive SHA-256: `3a0a129e4e6af0311ad162177250fc8b63a5308b7ba3c6bdda827005b2463f98`
- Concatenated Base64 SHA-256: `b139b00556f262c1fe782d1c2798815ad39460344e809000d226760048e763d8`

The eight numbered parts must be concatenated in lexical order. The workflow verifies every part, the concatenated Base64 stream, and the decoded source archive before extraction.

## 2Captcha privacy boundary

2Captcha support is optional and disabled by default. When enabled, the application sends only:

- the user-provided 2Captcha API key;
- the public DCInside reCAPTCHA site key;
- the fixed generic page URL `https://gallog.dcinside.com/`;
- the task type and invisibility flag required by the 2Captcha API.

The application does not put the DCInside ID, password, login cookies, individual gallog URL, post number, post contents, proxy credentials, callback URL, or softId in a 2Captcha request. The optional API key is stored locally with AES-GCM under a non-exportable Android Keystore key. Android backup remains disabled.

The workflow runs unit tests, Android lint, an APK build, Manifest/permission/signing/DEX inspections, a source-level forbidden-surface gate, and uploads the APK together with the exact corresponding source archive and verification reports.
