package com.tokengourmet.dccleanersafe;

import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Set;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

public final class DcHtmlParser {
    private static final Pattern POST_NUMBER = Pattern.compile(
            "<li\\b[^>]*\\bdata-no\\s*=\\s*(?:[\\\"']([0-9]+)[\\\"']|([0-9]+))[^>]*>",
            Pattern.CASE_INSENSITIVE);
    private static final Pattern PAGE_NUMBER = Pattern.compile(
            "(?:[?&]|&amp;)p=([0-9]{1,6})(?=[&#\\\"']|$)",
            Pattern.CASE_INSENSITIVE);

    private DcHtmlParser() {
    }

    /** Returns the document order used by DCInside (normally newest first). */
    public static List<String> parsePostNumbers(String html) {
        Set<String> ordered = new LinkedHashSet<>();
        if (html == null || html.isEmpty()) {
            return new ArrayList<>();
        }

        String lower = html.toLowerCase(Locale.ROOT);
        int marker = lower.indexOf("cont_listbox");
        if (marker < 0) {
            return new ArrayList<>();
        }
        int start = lower.lastIndexOf('<', marker);
        if (start < 0) {
            start = marker;
        }
        int end = lower.indexOf("bottom_paging_box", marker);
        String listSection = end > start ? html.substring(start, end) : html.substring(start);

        Matcher matcher = POST_NUMBER.matcher(listSection);
        while (matcher.find()) {
            String number = matcher.group(1) != null ? matcher.group(1) : matcher.group(2);
            ordered.add(number);
        }
        return new ArrayList<>(ordered);
    }

    public static int parseMaxPage(String html) {
        int max = 1;
        if (html == null || html.isEmpty()) {
            return max;
        }
        Matcher matcher = PAGE_NUMBER.matcher(html);
        while (matcher.find()) {
            try {
                max = Math.max(max, Integer.parseInt(matcher.group(1)));
            } catch (NumberFormatException ignored) {
                // Ignore malformed pagination values.
            }
        }
        return max;
    }

    public static boolean hasListContainer(String html) {
        return html != null && html.toLowerCase(Locale.ROOT).contains("cont_listbox");
    }

    public static boolean looksLikeLoginPage(String html) {
        if (html == null) {
            return false;
        }
        String lower = html.toLowerCase(Locale.ROOT);
        return lower.contains("login/member_check")
                || lower.contains("name=\"user_id\"")
                || lower.contains("name='user_id'")
                || lower.contains("로그인이 필요")
                || lower.contains("로그인 후 이용");
    }

    public static boolean looksBlocked(String html) {
        if (html == null) {
            return false;
        }
        String lower = html.toLowerCase(Locale.ROOT);
        return lower.contains("접속이 차단")
                || lower.contains("비정상적인 접근")
                || lower.contains("too many requests")
                || lower.contains("access denied");
    }

    public static boolean looksLikeCaptchaGate(String html) {
        if (html == null) {
            return false;
        }
        String lower = html.toLowerCase(Locale.ROOT);
        boolean marker = lower.contains("g-recaptcha")
                || lower.contains("recaptcha")
                || lower.contains("자동입력 방지");
        return marker && !hasListContainer(html);
    }
}
