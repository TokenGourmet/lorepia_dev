package com.tokengourmet.dccleanersafe;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

public final class CookieUtils {
    private CookieUtils() {
    }

    public static String findCookie(String cookieHeader, String name) {
        if (cookieHeader == null || name == null || name.isEmpty()) {
            return null;
        }
        String[] parts = cookieHeader.split(";");
        for (String part : parts) {
            String trimmed = part.trim();
            int separator = trimmed.indexOf('=');
            if (separator <= 0) {
                continue;
            }
            if (name.equals(trimmed.substring(0, separator).trim())) {
                return trimmed.substring(separator + 1);
            }
        }
        return null;
    }

    public static String fingerprint(String cookieHeader) {
        List<String> normalized = new ArrayList<>();
        if (cookieHeader != null) {
            String[] parts = cookieHeader.split(";");
            for (String part : parts) {
                String trimmed = part.trim();
                int separator = trimmed.indexOf('=');
                if (separator > 0) {
                    String name = trimmed.substring(0, separator).trim();
                    String value = trimmed.substring(separator + 1);
                    if (!name.isEmpty()) {
                        normalized.add(name + "=" + value);
                    }
                }
            }
        }
        Collections.sort(normalized);
        return sha256(String.join("\n", normalized));
    }

    private static String sha256(String value) {
        try {
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            byte[] hash = digest.digest(value.getBytes(StandardCharsets.UTF_8));
            StringBuilder out = new StringBuilder(hash.length * 2);
            for (byte b : hash) {
                out.append(String.format("%02x", b & 0xff));
            }
            return out.toString();
        } catch (NoSuchAlgorithmException impossible) {
            throw new IllegalStateException("SHA-256 unavailable", impossible);
        }
    }
}
