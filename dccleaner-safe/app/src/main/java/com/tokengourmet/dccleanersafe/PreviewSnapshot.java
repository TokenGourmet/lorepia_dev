package com.tokengourmet.dccleanersafe;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

public final class PreviewSnapshot {
    public final String userId;
    public final String type;
    public final int totalDiscovered;
    public final List<String> postNumbers;
    public final String cookieFingerprint;
    public final long createdAtMillis;

    public PreviewSnapshot(
            String userId,
            String type,
            int totalDiscovered,
            List<String> postNumbers,
            String cookieFingerprint,
            long createdAtMillis) {
        this.userId = userId;
        this.type = type;
        this.totalDiscovered = totalDiscovered;
        this.postNumbers = Collections.unmodifiableList(new ArrayList<>(postNumbers));
        this.cookieFingerprint = cookieFingerprint;
        this.createdAtMillis = createdAtMillis;
    }
}
