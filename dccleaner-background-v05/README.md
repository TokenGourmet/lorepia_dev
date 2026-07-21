# DC Cleaner Background 0.5 CI bootstrap

This directory transports the independently assembled Android source archive into an isolated GitHub Actions build.

## Pinned source

- Exact source archive: `DC-Cleaner-Background-0.5.0-source.tar.gz`
- Source archive SHA-256: `32b08817a0d50943dfcd2f664f41dd7016068f709a1756d2f4e5b4370b3084d9`
- Concatenated Base64 SHA-256: `609958c106b1b8d3140e2e768155e9d0f7be9554ae7388f25b7a069bb1a1941e`

Chunks, in exact concatenation order:

1. `source.tar.gz.b64.part-00` — `c7547b8b762a71eeeb1400cd687254a2e50d3503a1a095a57c3b29ac0ce9a27c`
2. `source.tar.gz.b64.part-01` — `e407fb8a35587c3ba397178aeb66e974e8e0208b722cccfb622cfd06196743cd`
3. `source.tar.gz.b64.part-02` — `3283b8c8de0a8dcacc5613e6edb07248df52d6bca035be0f36e18a1a07c768ce`
4. `source.tar.gz.b64.part-03` — `3e0ba4d9bcc5f470a0cbab6ef5502e7849ba9135dd71d1230b8f2e1f5bf888c2`
5. `source.tar.gz.b64.tail-00` — `edb113670a2cd27a9dac5d05611fb963d6c6a13cd2cdf386c93e79cfc23ff5e0`
6. `source.tar.gz.b64.tail-01` — `b217d3b9e5a5ba06a3e1283ce063512624cd826eec89f35dd2e57380e344ca5c`
7. `source.tar.gz.b64.tail-02` — `cab9e356a10ca6b4356b08e58c0ad3249f20ef1bea506413937e9d3149d5d5a1`
8. `source.tar.gz.b64.tail-03` — `f2c3cf699780f87f3a85ad876c91b312dca388bc6e8f875a04306011cb15f275`
9. `source.tar.gz.b64.tail-04` — `25b4e65e78dc19a1a067943ae503def705688174cfe36b4da9ef5d16b2258f56`
10. `source.tar.gz.b64.tail-05` — `714fb39e38b022bf35710ab6844fbb9be3e4a6a1931502410fd0c335f71e9930`
11. `source.tar.gz.b64.tail-06` — `e6520ef1f539ec2f64c25836c4eec74335a0f8127e9666f6c0b337c31211a12b`
12. `source.tar.gz.b64.tail-07` — `9cad6b2549674f940ea45ada7ddc40aab822b88cccdfbcb1b0e02cc42cb53e92`

## Functional boundary

- Automatic form fill and submit happens only on the exact official `https://sign.dcinside.com/login` origin and path.
- The application clears the previous WebView session before automatic login to avoid operating on the wrong account.
- Preview and deletion run in a non-exported `dataSync` foreground service started by explicit user action.
- A visible notification and stop action remain while the background task is active.
- A partial wake lock is held only inside the foreground service, with an explicit six-hour maximum.
- The preview cookie and optional 2Captcha key are encrypted under Android Keystore and deleted when the task completes, stops, or fails.
- Optional 2Captcha requests exclude the DCInside ID, password, cookies, CSRF value, post numbers, content, and target list.
- No boot receiver, exact alarm, accessibility service, analytics SDK, advertisement SDK, native library, or dynamic-code loader is present.

The workflow verifies all transport hashes before extraction, then runs unit tests, Android lint, APK compilation, permission/Manifest/signing/DEX checks, and privacy-oriented source gates before publishing the test artifact.
