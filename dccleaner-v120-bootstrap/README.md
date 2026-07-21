# DC Cleaner 1.2 CI bootstrap

This directory transports the reviewed DC Cleaner 1.2 Android source archive into an isolated GitHub Actions build.

## Pinned source

- Exact source archive: `DC-Cleaner-1.2.0-source.tar.gz`
- Source archive SHA-256: `ff7e8c15032ba938b0f0f490d7d2d41a2f5538068b328070513843a316f64e32`
- Concatenated Base64 SHA-256: `140c1bee246408b91ef1accfea17b63714f04001df0fe11d48e1776e7943c2bf`

Transport files, in exact concatenation order:

1. `source.tar.gz.b64.part-00` — `edb20b0eb51577e3d79bc6e321354f999cd36a534be4687be3d5c414751453f5`
2. `source.tar.gz.b64.part-01` — `7a79ca2e8715bb39853a59fd74a13dd454e52abfa6586962c0e17a0e26ce71ac`
3. `source.tar.gz.b64.part-02` — `1f315b8f4863db8ac12ccd528a2850b669c37d67b188efcf933d97fe89922af3`
4. `source.tar.gz.b64.part-03-04` — `218bdc148697f87de1e56b408b6647ef2d96ecf28496084ff3445c4dcf57da99`
5. `source.tar.gz.b64.part-05-06` — `198f003d1a4c3fc3e0184c51296599ebdbeec08cb969bb0fd7ae16aa8859470a`
6. `source.tar.gz.b64.part-07-08` — `69f578d9baaf3b72161d505e75414a1a14aee2418291523647ae0fe0e0e6333b`
7. `source.tar.gz.b64.part-09-10` — `159888a2da4e0c530645da3dec28e403af265559ced59dc0632dccd998124181`
8. `source.tar.gz.b64.part-11-12` — `c5c0b41effa3150e296792a8c11d354937ccdf9a57f8bb62fe974bf5f5bd255d`

## Login-cookie correction

Version 1.1 incorrectly treated the presence of the `ci_c` cookie on one gallog-domain lookup as the login oracle. DCInside can complete authentication before that cookie is exposed, can scope cookies to another DCInside host, and can issue or refresh cookies after the first gallog request.

Version 1.2 instead:

- merges WebView cookies from the verified DCInside root, `www`, `sign`, `msign`, gallog root, and the requested account gallog;
- merges native `Set-Cookie` responses back into the session jar;
- verifies the authenticated account through a real gallog response;
- rejects a verified account that differs from the requested identification code;
- treats `ci_c` as an optional deletion CSRF helper rather than proof that login succeeded;
- keeps deletion fail-closed unless a valid immutable preview is produced.

## Interface redesign

The old form screen was replaced with a product-style dashboard: dark gradient hero, four-step flow, account/range/2Captcha cards, segmented controls, a live operations console with metrics and progress, and a visually separated destructive action. Screenshot protection, local encrypted storage, foreground-service deletion, and typed confirmation remain.

The workflow verifies all hashes before extraction, then runs unit tests, Android lint, APK compilation, permission/Manifest/signing/DEX checks, native-library checks, and privacy-oriented source gates before publishing the APK and exact corresponding source archive.
