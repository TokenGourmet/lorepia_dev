package com.tokengourmet.dccleanersafe;

import javax.net.ssl.HttpsURLConnection;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.io.UnsupportedEncodingException;
import java.net.URL;
import java.net.URLEncoder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Set;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import java.util.regex.Pattern;

public final class DcClient {
    private static final String GALLOG_ORIGIN = "https://gallog.dcinside.com";
    private static final String USER_AGENT =
            "Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 "
                    + "(KHTML, like Gecko) Chrome/125.0 Mobile Safari/537.36 "
                    + "DcCleanerSafe/0.1";
    private static final int CONNECT_TIMEOUT_MS = 20_000;
    private static final int READ_TIMEOUT_MS = 25_000;
    private static final int MAX_RESPONSE_BYTES = 2 * 1024 * 1024;
    private static final int MAX_PAGES = 500;
    private static final int MAX_ITEMS = 100_000;
    private static final int MAX_DELETE_LIMIT = 100_000;
    private static final long PAGE_DELAY_MS = 1_100L;
    private static final long DELETE_DELAY_MS = 2_000L;
    private static final long MAX_SNAPSHOT_AGE_MS = 10 * 60 * 1000L;
    private static final Pattern JSON_SUCCESS = Pattern.compile(
            "\\\"result\\\"\\s*:\\s*\\\"success\\\"",
            Pattern.CASE_INSENSITIVE);

    private final AtomicReference<HttpsURLConnection> activeConnection = new AtomicReference<>();

    public interface PreviewProgress {
        void onProgress(int processedPages, int totalPages, int discoveredCount);
    }

    public interface DeleteProgress {
        void onProgress(int processed, int total, int deleted, int failed, String message);
    }

    public void cancelActiveRequest() {
        HttpsURLConnection connection = activeConnection.get();
        if (connection != null) {
            connection.disconnect();
        }
    }

    public PreviewSnapshot preview(
            String rawUserId,
            String type,
            String cookieHeader,
            int maxDeleteItems,
            AtomicBoolean cancelled,
            PreviewProgress progress) throws IOException, InterruptedException {
        String userId = SafeUrlPolicy.normalizeUserId(rawUserId);
        String normalizedType = normalizeType(type);
        requireSessionCookie(cookieHeader);
        if (maxDeleteItems < 1 || maxDeleteItems > MAX_DELETE_LIMIT) {
            throw new IllegalArgumentException("최대 삭제 개수는 1~100,000 사이여야 합니다.");
        }

        HttpResult firstPage = get(listUrl(userId, normalizedType, 1), cookieHeader);
        validateListResponse(firstPage);
        int totalPages = DcHtmlParser.parseMaxPage(firstPage.body);
        if (totalPages > MAX_PAGES) {
            throw new IOException("페이지가 안전 상한인 500개를 초과합니다. 범위를 나눠 처리하세요.");
        }

        Set<String> oldestFirst = new LinkedHashSet<>();
        int processedPages = 0;
        boolean madeAdditionalRequest = false;

        for (int page = totalPages; page >= 1; page--) {
            checkCancelled(cancelled);
            HttpResult result;
            if (page == 1) {
                result = firstPage;
            } else {
                if (madeAdditionalRequest || totalPages >= 1) {
                    Thread.sleep(PAGE_DELAY_MS);
                }
                madeAdditionalRequest = true;
                result = get(listUrl(userId, normalizedType, page), cookieHeader);
                validateListResponse(result);
            }

            List<String> pageItems = DcHtmlParser.parsePostNumbers(result.body);
            Collections.reverse(pageItems);
            oldestFirst.addAll(pageItems);
            if (oldestFirst.size() > MAX_ITEMS) {
                throw new IOException("대상이 안전 상한인 100,000개를 초과합니다. 범위를 나눠 처리하세요.");
            }
            processedPages++;
            progress.onProgress(processedPages, totalPages, oldestFirst.size());
        }

        List<String> all = new ArrayList<>(oldestFirst);
        int selectedCount = Math.min(maxDeleteItems, all.size());
        List<String> selected = new ArrayList<>(all.subList(0, selectedCount));

        return new PreviewSnapshot(
                userId,
                normalizedType,
                all.size(),
                selected,
                CookieUtils.fingerprint(cookieHeader),
                System.currentTimeMillis());
    }

    public DeleteReport deleteSnapshot(
            PreviewSnapshot snapshot,
            String cookieHeader,
            AtomicBoolean cancelled,
            DeleteProgress progress) throws IOException, InterruptedException {
        if (snapshot == null) {
            throw new IllegalArgumentException("먼저 미리보기를 실행하세요.");
        }
        long age = System.currentTimeMillis() - snapshot.createdAtMillis;
        if (age < 0 || age > MAX_SNAPSHOT_AGE_MS) {
            throw new IOException("미리보기가 10분 이상 지났습니다. 목록을 다시 확인하세요.");
        }
        requireSessionCookie(cookieHeader);
        if (!snapshot.cookieFingerprint.equals(CookieUtils.fingerprint(cookieHeader))) {
            throw new IOException("미리보기 이후 로그인 세션이 바뀌었습니다. 다시 미리보기 하세요.");
        }
        String csrfCookie = CookieUtils.findCookie(cookieHeader, "ci_c");
        if (csrfCookie == null || csrfCookie.isEmpty()) {
            throw new IOException("삭제에 필요한 ci_c 세션 쿠키가 없습니다. 웹 화면에서 다시 로그인하세요.");
        }

        int deleted = 0;
        int alreadyDeleted = 0;
        int failed = 0;
        int consecutiveFailures = 0;
        List<String> failureMessages = new ArrayList<>();
        int total = snapshot.postNumbers.size();

        for (int index = 0; index < total; index++) {
            checkCancelled(cancelled);
            String postNo = snapshot.postNumbers.get(index);
            HttpResult result = post(snapshot, cookieHeader, csrfCookie, postNo);
            DeleteClassification classification = classifyDelete(result);

            switch (classification) {
                case SUCCESS:
                    deleted++;
                    consecutiveFailures = 0;
                    break;
                case ALREADY_DELETED:
                    alreadyDeleted++;
                    consecutiveFailures = 0;
                    break;
                case CAPTCHA:
                    failureMessages.add("캡챠가 요구되어 즉시 중단했습니다.");
                    progress.onProgress(index, total, deleted, failed, "캡챠 감지 — 중단");
                    return new DeleteReport(deleted, alreadyDeleted, failed, true, failureMessages);
                case AUTH_EXPIRED:
                    failed++;
                    failureMessages.add("로그인 세션이 만료되었거나 접근이 거부되었습니다.");
                    progress.onProgress(index + 1, total, deleted, failed, "세션 만료 — 중단");
                    return new DeleteReport(deleted, alreadyDeleted, failed, false, failureMessages);
                case RATE_LIMITED:
                    failed++;
                    failureMessages.add("서버가 요청을 제한했습니다(HTTP " + result.code + ").");
                    progress.onProgress(index + 1, total, deleted, failed, "요청 제한 — 중단");
                    return new DeleteReport(deleted, alreadyDeleted, failed, false, failureMessages);
                case FAILED:
                    failed++;
                    consecutiveFailures++;
                    failureMessages.add("항목 " + postNo + " 삭제 실패(HTTP " + result.code + ").");
                    if (consecutiveFailures >= 3) {
                        progress.onProgress(index + 1, total, deleted, failed, "연속 실패 3회 — 중단");
                        return new DeleteReport(deleted, alreadyDeleted, failed, false, failureMessages);
                    }
                    break;
            }

            progress.onProgress(
                    index + 1,
                    total,
                    deleted,
                    failed,
                    "처리 " + (index + 1) + "/" + total);

            if (index + 1 < total) {
                Thread.sleep(DELETE_DELAY_MS);
            }
        }

        return new DeleteReport(deleted, alreadyDeleted, failed, false, failureMessages);
    }

    private static void validateListResponse(HttpResult result) throws IOException {
        if (result.code == 401 || result.code == 403 || isRedirect(result.code)) {
            throw new IOException("로그인 세션이 없거나 접근이 거부되었습니다(HTTP " + result.code + ").");
        }
        if (result.code == 429) {
            throw new IOException("요청이 너무 많아 서버가 제한했습니다. 나중에 다시 시도하세요.");
        }
        if (result.code < 200 || result.code >= 300) {
            throw new IOException("목록 요청 실패(HTTP " + result.code + ").");
        }
        if (DcHtmlParser.looksLikeLoginPage(result.body)) {
            throw new IOException("로그인 페이지가 반환되었습니다. 아래 웹 화면에서 로그인하세요.");
        }
        if (DcHtmlParser.looksBlocked(result.body)) {
            throw new IOException("접근 차단 페이지가 감지되었습니다. 자동 처리를 시작하지 않았습니다.");
        }
        if (DcHtmlParser.looksLikeCaptchaGate(result.body)) {
            throw new IOException("캡챠 페이지가 감지되었습니다. 이 앱은 캡챠를 우회하지 않습니다.");
        }
        if (!DcHtmlParser.hasListContainer(result.body)) {
            throw new IOException("예상한 갤로그 목록 구조를 찾지 못했습니다. 사이트 변경 가능성이 있어 중단합니다.");
        }
    }

    private static void requireSessionCookie(String cookieHeader) throws IOException {
        if (cookieHeader == null || cookieHeader.trim().isEmpty()) {
            throw new IOException("디시인사이드 로그인 쿠키가 없습니다. 아래 웹 화면에서 로그인하세요.");
        }
    }

    private static String normalizeType(String type) {
        if ("posting".equals(type) || "comment".equals(type)) {
            return type;
        }
        throw new IllegalArgumentException("지원하지 않는 삭제 유형입니다.");
    }

    private static String listUrl(String userId, String type, int page) {
        return GALLOG_ORIGIN + "/" + userId + "/" + type + "/index?p=" + page;
    }

    private HttpResult get(String url, String cookieHeader) throws IOException {
        HttpsURLConnection connection = open(url, cookieHeader);
        connection.setRequestMethod("GET");
        connection.setRequestProperty("Accept", "text/html,application/xhtml+xml");
        connection.setRequestProperty("Accept-Language", "ko-KR,ko;q=0.9,en;q=0.6");
        return execute(connection, null);
    }

    private HttpResult post(
            PreviewSnapshot snapshot,
            String cookieHeader,
            String csrfCookie,
            String postNo) throws IOException {
        String endpoint = GALLOG_ORIGIN + "/" + snapshot.userId + "/ajax/log_list_ajax/delete";
        HttpsURLConnection connection = open(endpoint, cookieHeader);
        connection.setRequestMethod("POST");
        connection.setDoOutput(true);
        connection.setRequestProperty("Accept", "application/json, text/javascript, */*; q=0.01");
        connection.setRequestProperty("Accept-Language", "ko-KR,ko;q=0.9,en;q=0.6");
        connection.setRequestProperty("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8");
        connection.setRequestProperty("Origin", GALLOG_ORIGIN);
        connection.setRequestProperty(
                "Referer",
                GALLOG_ORIGIN + "/" + snapshot.userId + "/" + snapshot.type);
        connection.setRequestProperty("X-Requested-With", "XMLHttpRequest");

        String body = "ci_t=" + encode(csrfCookie)
                + "&no=" + encode(postNo)
                + "&service_code=" + encode("undefined");
        return execute(connection, body.getBytes(StandardCharsets.UTF_8));
    }

    private static HttpsURLConnection open(String url, String cookieHeader) throws IOException {
        if (!SafeUrlPolicy.isAllowedApiUrl(url)) {
            throw new IOException("허용되지 않은 네트워크 대상입니다.");
        }
        HttpsURLConnection connection = (HttpsURLConnection) new URL(url).openConnection();
        connection.setInstanceFollowRedirects(false);
        connection.setConnectTimeout(CONNECT_TIMEOUT_MS);
        connection.setReadTimeout(READ_TIMEOUT_MS);
        connection.setUseCaches(false);
        connection.setRequestProperty("User-Agent", USER_AGENT);
        connection.setRequestProperty("Cookie", cookieHeader);
        return connection;
    }

    private HttpResult execute(HttpsURLConnection connection, byte[] requestBody) throws IOException {
        activeConnection.set(connection);
        try {
            if (requestBody != null) {
                connection.setFixedLengthStreamingMode(requestBody.length);
                try (OutputStream output = connection.getOutputStream()) {
                    output.write(requestBody);
                }
            }
            int code = connection.getResponseCode();
            InputStream input = code >= 400 ? connection.getErrorStream() : connection.getInputStream();
            String body = input == null ? "" : readLimited(input);
            return new HttpResult(code, body);
        } finally {
            activeConnection.compareAndSet(connection, null);
            connection.disconnect();
        }
    }

    private static String readLimited(InputStream input) throws IOException {
        try (InputStream source = input; ByteArrayOutputStream out = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[8192];
            int total = 0;
            int read;
            while ((read = source.read(buffer)) != -1) {
                total += read;
                if (total > MAX_RESPONSE_BYTES) {
                    throw new IOException("서버 응답이 안전 상한을 초과했습니다.");
                }
                out.write(buffer, 0, read);
            }
            return out.toString(StandardCharsets.UTF_8.name());
        }
    }

    private static String encode(String value) {
        try {
            return URLEncoder.encode(value, "UTF-8");
        } catch (UnsupportedEncodingException impossible) {
            throw new IllegalStateException("UTF-8 unavailable", impossible);
        }
    }

    private static void checkCancelled(AtomicBoolean cancelled) throws InterruptedException {
        if (cancelled.get() || Thread.currentThread().isInterrupted()) {
            throw new InterruptedException("사용자가 작업을 중단했습니다.");
        }
    }

    private static boolean isRedirect(int code) {
        return code >= 300 && code < 400;
    }

    private static DeleteClassification classifyDelete(HttpResult result) {
        if (result.code == 401 || result.code == 403 || isRedirect(result.code)) {
            return DeleteClassification.AUTH_EXPIRED;
        }
        if (result.code == 429) {
            return DeleteClassification.RATE_LIMITED;
        }
        String lower = result.body.toLowerCase(Locale.ROOT);
        if (lower.contains("g-recaptcha")
                || lower.contains("recaptcha")
                || lower.contains("자동입력 방지")) {
            return DeleteClassification.CAPTCHA;
        }
        if (result.code >= 200 && result.code < 300 && JSON_SUCCESS.matcher(result.body).find()) {
            return DeleteClassification.SUCCESS;
        }
        if (result.body.contains("글 번호가 올바르지 않습니다")) {
            return DeleteClassification.ALREADY_DELETED;
        }
        return DeleteClassification.FAILED;
    }

    private enum DeleteClassification {
        SUCCESS,
        ALREADY_DELETED,
        CAPTCHA,
        AUTH_EXPIRED,
        RATE_LIMITED,
        FAILED
    }

    private static final class HttpResult {
        final int code;
        final String body;

        HttpResult(int code, String body) {
            this.code = code;
            this.body = body;
        }
    }

    public static final class DeleteReport {
        public final int deleted;
        public final int alreadyDeleted;
        public final int failed;
        public final boolean captchaStopped;
        public final List<String> failureMessages;

        DeleteReport(
                int deleted,
                int alreadyDeleted,
                int failed,
                boolean captchaStopped,
                List<String> failureMessages) {
            this.deleted = deleted;
            this.alreadyDeleted = alreadyDeleted;
            this.failed = failed;
            this.captchaStopped = captchaStopped;
            this.failureMessages = Collections.unmodifiableList(new ArrayList<>(failureMessages));
        }
    }
}
