# DC Cleaner Safe 0.2 CI bootstrap

This directory transports the reviewed DC Cleaner Safe 0.2 source archive into an isolated GitHub Actions Android build.

## Pinned source

- Exact source archive: `DC-Cleaner-Safe-0.2.0-source.tar.gz`
- Source archive SHA-256: `a8860922826addf8eee93eff890826ade76019e6758969c45c062038b86461cd`
- Concatenated Base64 SHA-256: `2ff53f67fa77c50ae8112d602004034a9752d385fa173d78dd945d61120c2b6d`

Parts, in exact concatenation order:

1. `source.tar.gz.b64.part-00` — `e578f6fe013b9663ffb40bb87f9aa60386ce5833089bf578605eeffae5a29af6`
2. `source.tar.gz.b64.part-01` — `3627c1f1567145018a7c7aab0f95b469af9d934895aadc6baa3d5e85944c9fbc`

The workflow verifies both hashes before extraction. It then runs unit tests, Android lint, an APK build, Manifest/permission/signing/DEX inspections, a source-level forbidden-surface gate, and uploads the APK together with the exact corresponding source archive and reports.

## Privacy boundary

- Optional DCInside ID/password storage uses AES-GCM with a non-exportable Android Keystore key.
- Android backup is disabled.
- Login form filling happens only on the exact official `sign.dcinside.com/login` page and never submits automatically.
- There is no 2Captcha integration. CAPTCHA is solved manually on the official DCInside page, so account credentials and session cookies are not sent to a CAPTCHA provider.
