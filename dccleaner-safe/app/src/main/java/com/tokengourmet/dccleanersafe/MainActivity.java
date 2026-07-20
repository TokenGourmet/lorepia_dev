package com.tokengourmet.dccleanersafe;

import android.app.Activity;
import android.app.AlertDialog;
import android.graphics.Color;
import android.net.Uri;
import android.net.http.SslError;
import android.os.Bundle;
import android.text.Editable;
import android.text.InputType;
import android.text.TextWatcher;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowManager;
import android.webkit.CookieManager;
import android.webkit.SslErrorHandler;
import android.webkit.WebResourceRequest;
import android.webkit.WebSettings;
import android.webkit.WebStorage;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.ArrayAdapter;
import android.widget.Button;
import android.widget.CheckBox;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.Spinner;
import android.widget.TextView;
import android.widget.Toast;

import java.text.DateFormat;
import java.util.Date;
import java.util.Locale;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;

public final class MainActivity extends Activity {
    private static final String DC_HOME = "https://www.dcinside.com/";
    private static final String GALLOG_COOKIE_URL = "https://gallog.dcinside.com/";

    private final ExecutorService worker = Executors.newSingleThreadExecutor();
    private final AtomicBoolean cancelled = new AtomicBoolean(false);
    private final DcClient client = new DcClient();

    private WebView webView;
    private EditText userIdInput;
    private EditText maxItemsInput;
    private Spinner typeSpinner;
    private Button homeButton;
    private Button gallogButton;
    private Button previewButton;
    private Button deleteButton;
    private Button stopButton;
    private Button clearButton;
    private ProgressBar progressBar;
    private TextView statusView;
    private CheckBox clearOnExit;

    private volatile PreviewSnapshot previewSnapshot;
    private Operation operation = Operation.IDLE;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(buildUi());
        configureWebView();
        attachInvalidationListeners();
        webView.loadUrl(DC_HOME);
        appendStatus("아래 공식 디시인사이드 웹 화면에서 직접 로그인하세요. 앱은 비밀번호 입력란을 만들거나 저장하지 않습니다.");
    }

    private View buildUi() {
        int padding = dp(12);
        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setPadding(padding, padding, padding, padding);
        root.setBackgroundColor(Color.rgb(247, 247, 247));

        TextView title = new TextView(this);
        title.setText("DC Cleaner Safe — 삭제 전용");
        title.setTextSize(20f);
        title.setTextColor(Color.BLACK);
        title.setPadding(0, 0, 0, dp(6));
        root.addView(title, matchWrap());

        TextView privacy = new TextView(this);
        privacy.setText(
                "비밀번호·2Captcha 키를 저장하지 않습니다. 글쓰기, 댓글쓰기, 광고, 분석 SDK, 백그라운드 상주 기능도 없습니다.");
        privacy.setTextColor(Color.DKGRAY);
        privacy.setTextSize(13f);
        privacy.setPadding(0, 0, 0, dp(6));
        root.addView(privacy, matchWrap());

        LinearLayout browserRow = horizontalRow();
        homeButton = button("디시 홈/로그인");
        homeButton.setOnClickListener(v -> webView.loadUrl(DC_HOME));
        browserRow.addView(homeButton, weighted());

        gallogButton = button("내 갤로그 열기");
        gallogButton.setOnClickListener(v -> openGallog());
        browserRow.addView(gallogButton, weighted());
        root.addView(browserRow, matchWrap());

        LinearLayout accountRow = horizontalRow();
        userIdInput = new EditText(this);
        userIdInput.setHint("디시 아이디");
        userIdInput.setSingleLine(true);
        userIdInput.setInputType(
                InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD);
        accountRow.addView(
                userIdInput,
                new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 2f));

        typeSpinner = new Spinner(this);
        ArrayAdapter<String> typeAdapter = new ArrayAdapter<>(
                this,
                android.R.layout.simple_spinner_item,
                new String[]{"게시글", "댓글"});
        typeAdapter.setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item);
        typeSpinner.setAdapter(typeAdapter);
        accountRow.addView(
                typeSpinner,
                new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
        root.addView(accountRow, matchWrap());

        LinearLayout limitRow = horizontalRow();
        TextView limitLabel = new TextView(this);
        limitLabel.setText("오래된 항목부터 최대 삭제 개수");
        limitLabel.setTextColor(Color.DKGRAY);
        limitLabel.setGravity(android.view.Gravity.CENTER_VERTICAL);
        limitRow.addView(
                limitLabel,
                new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 2f));

        maxItemsInput = new EditText(this);
        maxItemsInput.setText("10");
        maxItemsInput.setSingleLine(true);
        maxItemsInput.setInputType(InputType.TYPE_CLASS_NUMBER);
        limitRow.addView(
                maxItemsInput,
                new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
        root.addView(limitRow, matchWrap());

        LinearLayout actionRow = horizontalRow();
        previewButton = button("1. 삭제 대상 미리보기");
        previewButton.setOnClickListener(v -> startPreview());
        actionRow.addView(previewButton, weighted());

        deleteButton = button("2. 확인 후 삭제");
        deleteButton.setEnabled(false);
        deleteButton.setOnClickListener(v -> showDeleteConfirmation());
        actionRow.addView(deleteButton, weighted());
        root.addView(actionRow, matchWrap());

        LinearLayout stopRow = horizontalRow();
        stopButton = button("즉시 중단");
        stopButton.setEnabled(false);
        stopButton.setOnClickListener(v -> requestCancellation("사용자가 중단을 요청했습니다."));
        stopRow.addView(stopButton, weighted());

        clearButton = button("세션·웹데이터 삭제");
        clearButton.setOnClickListener(v -> clearWebSession());
        stopRow.addView(clearButton, weighted());
        root.addView(stopRow, matchWrap());

        clearOnExit = new CheckBox(this);
        clearOnExit.setText("앱을 정상 종료할 때 WebView 로그인 세션 삭제");
        clearOnExit.setChecked(true);
        root.addView(clearOnExit, matchWrap());

        progressBar = new ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal);
        progressBar.setMax(100);
        progressBar.setProgress(0);
        root.addView(progressBar, matchWrap());

        statusView = new TextView(this);
        statusView.setTextSize(12f);
        statusView.setTextColor(Color.rgb(30, 30, 30));
        statusView.setBackgroundColor(Color.WHITE);
        statusView.setPadding(dp(8), dp(8), dp(8), dp(8));
        statusView.setTextIsSelectable(true);
        root.addView(
                statusView,
                new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(112)));

        webView = new WebView(this);
        root.addView(
                webView,
                new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));
        return root;
    }

    private void configureWebView() {
        WebView.setWebContentsDebuggingEnabled(false);
        WebSettings settings = webView.getSettings();
        settings.setJavaScriptEnabled(true);
        settings.setDomStorageEnabled(true);
        settings.setAllowFileAccess(false);
        settings.setAllowContentAccess(false);
        settings.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
        settings.setJavaScriptCanOpenWindowsAutomatically(false);
        settings.setSupportMultipleWindows(false);
        settings.setMediaPlaybackRequiresUserGesture(true);
        settings.setGeolocationEnabled(false);
        settings.setSafeBrowsingEnabled(true);
        settings.setSaveFormData(false);

        CookieManager cookieManager = CookieManager.getInstance();
        cookieManager.setAcceptCookie(true);
        cookieManager.setAcceptThirdPartyCookies(webView, false);

        webView.setDownloadListener((url, userAgent, contentDisposition, mimeType, contentLength) ->
                Toast.makeText(this, "웹 다운로드는 안전상 차단했습니다.", Toast.LENGTH_SHORT).show());

        webView.setWebViewClient(new WebViewClient() {
            @Override
            public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                if (!request.isForMainFrame()) {
                    return false;
                }
                return blockIfDisallowed(request.getUrl().toString());
            }

            @Override
            @SuppressWarnings("deprecation")
            public boolean shouldOverrideUrlLoading(WebView view, String url) {
                return blockIfDisallowed(url);
            }

            @Override
            public void onReceivedSslError(WebView view, SslErrorHandler handler, SslError error) {
                handler.cancel();
                appendStatus("TLS 인증서 오류로 페이지를 차단했습니다. 인증서 경고를 우회하지 않습니다.");
            }

            @Override
            public void onPageFinished(WebView view, String url) {
                appendStatus("웹 페이지: " + safeLocation(url));
            }
        });
    }

    private boolean blockIfDisallowed(String url) {
        if (SafeUrlPolicy.isAllowedTopLevelUrl(url)) {
            return false;
        }
        appendStatus("허용되지 않은 외부 이동을 차단했습니다: " + safeLocation(url));
        return true;
    }

    private void attachInvalidationListeners() {
        TextWatcher watcher = new TextWatcher() {
            @Override
            public void beforeTextChanged(CharSequence s, int start, int count, int after) {
            }

            @Override
            public void onTextChanged(CharSequence s, int start, int before, int count) {
                invalidatePreview("입력값이 바뀌어 기존 미리보기를 폐기했습니다.");
            }

            @Override
            public void afterTextChanged(Editable s) {
            }
        };
        userIdInput.addTextChangedListener(watcher);
        maxItemsInput.addTextChangedListener(watcher);
        typeSpinner.setOnItemSelectedListener(new android.widget.AdapterView.OnItemSelectedListener() {
            @Override
            public void onItemSelected(
                    android.widget.AdapterView<?> parent,
                    View view,
                    int position,
                    long id) {
                invalidatePreview("삭제 유형이 바뀌어 기존 미리보기를 폐기했습니다.");
            }

            @Override
            public void onNothingSelected(android.widget.AdapterView<?> parent) {
            }
        });
    }

    private void openGallog() {
        String userId;
        try {
            userId = SafeUrlPolicy.normalizeUserId(userIdInput.getText().toString());
        } catch (IllegalArgumentException e) {
            showMessage(e.getMessage());
            return;
        }
        webView.loadUrl("https://gallog.dcinside.com/" + userId + "/" + selectedType());
    }

    private void startPreview() {
        if (operation != Operation.IDLE) {
            showMessage("이미 작업 중입니다.");
            return;
        }

        final String userId;
        final int maxItems;
        try {
            userId = SafeUrlPolicy.normalizeUserId(userIdInput.getText().toString());
            maxItems = parseMaxItems();
        } catch (IllegalArgumentException e) {
            showMessage(e.getMessage());
            return;
        }
        final String type = selectedType();
        CookieManager.getInstance().flush();
        final String cookies = CookieManager.getInstance().getCookie(GALLOG_COOKIE_URL);
        if (cookies == null || cookies.trim().isEmpty()) {
            showMessage("먼저 아래 웹 화면에서 디시인사이드에 로그인하세요.");
            return;
        }

        webView.stopLoading();
        invalidatePreview(null);
        cancelled.set(false);
        setBusy(Operation.PREVIEW, true);
        appendStatus("미리보기 시작: " + displayType(type) + ". 아직 삭제하지 않습니다.");

        worker.submit(() -> {
            try {
                PreviewSnapshot snapshot = client.preview(
                        userId,
                        type,
                        cookies,
                        maxItems,
                        cancelled,
                        (current, total, count) -> runOnUiThread(() -> {
                            progressBar.setMax(Math.max(total, 1));
                            progressBar.setProgress(current);
                            statusView.setText(
                                    "미리보기 " + current + "/" + total + " 페이지 · 발견 " + count + "개");
                        }));
                runOnUiThread(() -> {
                    previewSnapshot = snapshot;
                    setBusy(Operation.PREVIEW, false);
                    deleteButton.setEnabled(!snapshot.postNumbers.isEmpty());
                    appendStatus(
                            "미리보기 완료: 전체 " + snapshot.totalDiscovered + "개 중 오래된 "
                                    + snapshot.postNumbers.size() + "개를 고정했습니다.");
                    showPreviewSummary(snapshot);
                });
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                runOnUiThread(() -> finishWithMessage("미리보기를 중단했습니다."));
            } catch (Exception e) {
                runOnUiThread(() -> {
                    if (cancelled.get()) {
                        finishWithMessage("미리보기를 중단했습니다.");
                    } else {
                        finishWithMessage("미리보기 실패: " + safeError(e));
                    }
                });
            }
        });
    }

    private void showPreviewSummary(PreviewSnapshot snapshot) {
        StringBuilder message = new StringBuilder();
        message.append(displayType(snapshot.type))
                .append(" 전체 ")
                .append(snapshot.totalDiscovered)
                .append("개를 찾았습니다.\n")
                .append("그중 오래된 ")
                .append(snapshot.postNumbers.size())
                .append("개만 이번 삭제 대상으로 고정했습니다.\n\n");

        int sample = Math.min(10, snapshot.postNumbers.size());
        if (sample > 0) {
            message.append("대상 번호 표본: ");
            for (int i = 0; i < sample; i++) {
                if (i > 0) {
                    message.append(", ");
                }
                message.append(snapshot.postNumbers.get(i));
            }
            message.append("\n\n");
        }
        message.append(
                "아직 아무것도 삭제하지 않았습니다. 미리보기는 10분 동안만 유효하며, 삭제 전에 정확한 확인 문구가 필요합니다.");

        new AlertDialog.Builder(this)
                .setTitle("미리보기 완료")
                .setMessage(message.toString())
                .setPositiveButton("확인", null)
                .show();
    }

    private void showDeleteConfirmation() {
        PreviewSnapshot snapshot = previewSnapshot;
        if (snapshot == null || snapshot.postNumbers.isEmpty()) {
            showMessage("삭제 가능한 미리보기가 없습니다.");
            return;
        }
        if (operation != Operation.IDLE) {
            showMessage("이미 작업 중입니다.");
            return;
        }

        String phrase = "삭제 " + snapshot.postNumbers.size();
        EditText confirmation = new EditText(this);
        confirmation.setSingleLine(true);
        confirmation.setHint(phrase);
        confirmation.setInputType(InputType.TYPE_CLASS_TEXT);

        int pad = dp(20);
        LinearLayout holder = new LinearLayout(this);
        holder.setPadding(pad, 0, pad, 0);
        holder.addView(confirmation, matchWrap());

        AlertDialog dialog = new AlertDialog.Builder(this)
                .setTitle("되돌릴 수 없는 삭제")
                .setMessage(
                        displayType(snapshot.type) + " " + snapshot.postNumbers.size()
                                + "개를 오래된 순서부터 삭제합니다.\n"
                                + "미리보기 이후 생성된 새 항목은 포함되지 않습니다.\n\n"
                                + "계속하려면 “" + phrase + "”를 정확히 입력하세요.")
                .setView(holder)
                .setNegativeButton("취소", null)
                .setPositiveButton("삭제 시작", null)
                .create();

        dialog.setOnShowListener(ignored ->
                dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener(v -> {
                    if (!phrase.equals(confirmation.getText().toString().trim())) {
                        confirmation.setError("확인 문구가 일치하지 않습니다.");
                        return;
                    }
                    dialog.dismiss();
                    startDelete(snapshot);
                }));
        dialog.show();
    }

    private void startDelete(PreviewSnapshot snapshot) {
        CookieManager.getInstance().flush();
        final String cookies = CookieManager.getInstance().getCookie(GALLOG_COOKIE_URL);
        if (cookies == null || cookies.trim().isEmpty()) {
            showMessage("로그인 세션이 사라졌습니다. 다시 로그인하고 미리보기부터 실행하세요.");
            invalidatePreview(null);
            return;
        }

        cancelled.set(false);
        setBusy(Operation.DELETE, true);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        progressBar.setMax(Math.max(snapshot.postNumbers.size(), 1));
        progressBar.setProgress(0);
        appendStatus("삭제 시작. 앱이 화면에서 사라지면 안전을 위해 작업을 중단합니다.");

        worker.submit(() -> {
            try {
                DcClient.DeleteReport report = client.deleteSnapshot(
                        snapshot,
                        cookies,
                        cancelled,
                        (processed, total, deleted, failed, message) -> runOnUiThread(() -> {
                            progressBar.setMax(Math.max(total, 1));
                            progressBar.setProgress(processed);
                            statusView.setText(
                                    message + " · 성공 " + deleted + " · 실패 " + failed);
                        }));
                runOnUiThread(() -> {
                    invalidatePreviewAfterDelete();
                    setBusy(Operation.DELETE, false);
                    String result = "삭제 성공 " + report.deleted
                            + "개\n이미 삭제됨 " + report.alreadyDeleted
                            + "개\n실패 " + report.failed + "개"
                            + (report.captchaStopped ? "\n캡챠 감지로 중단" : "");
                    appendStatus(result.replace('\n', ' '));
                    new AlertDialog.Builder(this)
                            .setTitle("작업 결과")
                            .setMessage(result)
                            .setPositiveButton("확인", null)
                            .show();
                });
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                runOnUiThread(() -> {
                    invalidatePreviewAfterDelete();
                    finishWithMessage("삭제 작업을 중단했습니다. 다시 미리보기하면 남은 항목을 확인할 수 있습니다.");
                });
            } catch (Exception e) {
                runOnUiThread(() -> {
                    invalidatePreviewAfterDelete();
                    if (cancelled.get()) {
                        finishWithMessage("삭제 작업을 중단했습니다. 다시 미리보기하세요.");
                    } else {
                        finishWithMessage("삭제 중단: " + safeError(e));
                    }
                });
            }
        });
    }

    private void requestCancellation(String message) {
        cancelled.set(true);
        client.cancelActiveRequest();
        appendStatus(message);
    }

    private void setBusy(Operation active, boolean busy) {
        operation = busy ? active : Operation.IDLE;
        previewButton.setEnabled(!busy);
        stopButton.setEnabled(busy);
        clearButton.setEnabled(!busy);
        homeButton.setEnabled(!busy);
        gallogButton.setEnabled(!busy);
        userIdInput.setEnabled(!busy);
        maxItemsInput.setEnabled(!busy);
        typeSpinner.setEnabled(!busy);
        webView.setEnabled(!busy);
        if (busy) {
            deleteButton.setEnabled(false);
        } else {
            deleteButton.setEnabled(
                    previewSnapshot != null && !previewSnapshot.postNumbers.isEmpty());
            getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        }
    }

    private void finishWithMessage(String message) {
        setBusy(operation, false);
        appendStatus(message);
        showMessage(message);
    }

    private void invalidatePreview(String message) {
        if (operation == Operation.DELETE) {
            return;
        }
        if (previewSnapshot != null && message != null) {
            appendStatus(message);
        }
        previewSnapshot = null;
        if (deleteButton != null) {
            deleteButton.setEnabled(false);
        }
    }

    private void invalidatePreviewAfterDelete() {
        previewSnapshot = null;
        if (deleteButton != null) {
            deleteButton.setEnabled(false);
        }
    }

    private void clearWebSession() {
        if (operation != Operation.IDLE) {
            showMessage("작업을 먼저 중단하세요.");
            return;
        }
        CookieManager.getInstance().removeAllCookies(success -> runOnUiThread(() -> {
            CookieManager.getInstance().flush();
            WebStorage.getInstance().deleteAllData();
            webView.clearCache(true);
            webView.clearFormData();
            webView.clearHistory();
            webView.loadUrl("about:blank");
            invalidatePreview(null);
            appendStatus("WebView 쿠키·저장소·캐시·기록을 삭제했습니다.");
        }));
    }

    private int parseMaxItems() {
        String raw = maxItemsInput.getText().toString().trim();
        try {
            int value = Integer.parseInt(raw);
            if (value < 1 || value > 100_000) {
                throw new NumberFormatException();
            }
            return value;
        } catch (NumberFormatException e) {
            throw new IllegalArgumentException("최대 삭제 개수는 1~100,000 사이의 숫자여야 합니다.");
        }
    }

    private String selectedType() {
        return typeSpinner.getSelectedItemPosition() == 0 ? "posting" : "comment";
    }

    private static String displayType(String type) {
        return "posting".equals(type) ? "게시글" : "댓글";
    }

    private void appendStatus(String message) {
        String time = DateFormat.getTimeInstance(DateFormat.MEDIUM, Locale.KOREA).format(new Date());
        String prior = statusView == null ? "" : statusView.getText().toString();
        String next = "[" + time + "] " + message + (prior.isEmpty() ? "" : "\n" + prior);
        if (next.length() > 5000) {
            next = next.substring(0, 5000);
        }
        if (statusView != null) {
            statusView.setText(next);
        }
    }

    private static String safeLocation(String raw) {
        try {
            Uri uri = Uri.parse(raw);
            String host = uri.getHost();
            String path = uri.getPath();
            return (host == null ? "내부 페이지" : host) + (path == null ? "" : path);
        } catch (Exception ignored) {
            return "알 수 없는 주소";
        }
    }

    private static String safeError(Exception error) {
        String message = error.getMessage();
        return message == null || message.trim().isEmpty()
                ? error.getClass().getSimpleName()
                : message;
    }

    private void showMessage(String message) {
        Toast.makeText(this, message, Toast.LENGTH_LONG).show();
    }

    private LinearLayout horizontalRow() {
        LinearLayout row = new LinearLayout(this);
        row.setOrientation(LinearLayout.HORIZONTAL);
        row.setPadding(0, 0, 0, dp(4));
        return row;
    }

    private Button button(String label) {
        Button button = new Button(this);
        button.setText(label);
        button.setAllCaps(false);
        return button;
    }

    private static LinearLayout.LayoutParams matchWrap() {
        return new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT);
    }

    private static LinearLayout.LayoutParams weighted() {
        return new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f);
    }

    private int dp(int value) {
        return Math.round(value * getResources().getDisplayMetrics().density);
    }

    @Override
    protected void onStop() {
        super.onStop();
        if (operation != Operation.IDLE) {
            requestCancellation("앱이 화면에서 사라져 작업 중단을 요청했습니다.");
        }
    }

    @Override
    protected void onDestroy() {
        cancelled.set(true);
        client.cancelActiveRequest();
        worker.shutdownNow();
        if (clearOnExit != null && clearOnExit.isChecked()) {
            CookieManager.getInstance().removeAllCookies(null);
            CookieManager.getInstance().flush();
            WebStorage.getInstance().deleteAllData();
        }
        if (webView != null) {
            webView.loadUrl("about:blank");
            webView.stopLoading();
            webView.clearHistory();
            webView.removeAllViews();
            webView.destroy();
        }
        super.onDestroy();
    }

    @Override
    @SuppressWarnings("deprecation")
    public void onBackPressed() {
        if (webView != null && webView.canGoBack()) {
            webView.goBack();
        } else {
            super.onBackPressed();
        }
    }

    private enum Operation {
        IDLE,
        PREVIEW,
        DELETE
    }
}
