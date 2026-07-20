package com.tokengourmet.dccleanersafe;

import java.net.URI;
import java.net.URISyntaxException;
import java.util.Locale;
import java.util.regex.Pattern;

public final class SafeUrlPolicy {
    private static final Pattern USER_ID = Pattern.compile("[A-Za-z0-9._-]{2,40}");

    private SafeUrlPolicy() {
    }

    public static boolean isValidUserId(String value) {
        return value != null && USER_ID.matcher(value.trim()).matches();
    }

    public static String normalizeUserId(String value) {
        if (!isValidUserId(value)) {
            throw new IllegalArgumentException(
                    "아이디는 영문, 숫자, 점, 밑줄, 하이픈으로 된 2~40자만 허용됩니다.");
        }
        return value.trim().toLowerCase(Locale.ROOT);
    }

    public static boolean isAllowedTopLevelUrl(String value) {
        if ("about:blank".equalsIgnoreCase(value)) {
            return true;
        }
        try {
            URI uri = new URI(value);
            if (!"https".equalsIgnoreCase(uri.getScheme())
                    || uri.getUserInfo() != null
                    || !isDefaultHttpsPort(uri.getPort())) {
                return false;
            }
            String host = uri.getHost();
            return host != null && isDcinsideHost(host);
        } catch (URISyntaxException e) {
            return false;
        }
    }

    public static boolean isAllowedApiUrl(String value) {
        try {
            URI uri = new URI(value);
            return "https".equalsIgnoreCase(uri.getScheme())
                    && uri.getUserInfo() == null
                    && isDefaultHttpsPort(uri.getPort())
                    && "gallog.dcinside.com".equalsIgnoreCase(uri.getHost());
        } catch (URISyntaxException e) {
            return false;
        }
    }

    private static boolean isDcinsideHost(String host) {
        String normalized = host.toLowerCase(Locale.ROOT);
        return normalized.equals("dcinside.com") || normalized.endsWith(".dcinside.com");
    }

    private static boolean isDefaultHttpsPort(int port) {
        return port == -1 || port == 443;
    }
}
