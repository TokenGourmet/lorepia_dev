package com.tokengourmet.dccleanersafe;

import org.junit.Test;

import java.util.Arrays;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

public class DcHtmlParserTest {
    @Test
    public void parsesOnlyScopedUniquePostNumbersInOrder() {
        String html = "<li data-no='999'></li>"
                + "<ul class='cont_listbox'>"
                + "<li class='x' data-no=\"123\"></li>"
                + "<li data-no='456'></li><li data-no=\"123\"></li>"
                + "</ul><div class='bottom_paging_box'></div>";
        assertEquals(Arrays.asList("123", "456"), DcHtmlParser.parsePostNumbers(html));
    }

    @Test
    public void parsesLargestPageAcrossEscapedLinks() {
        String html = "<a href='?p=2'>2</a><a href='?x=1&amp;p=27'>끝</a>";
        assertEquals(27, DcHtmlParser.parseMaxPage(html));
    }

    @Test
    public void extractsCookieWithoutPrefixCollision() {
        String cookies = "ci_c_extra=no; ci_c=secret-value; other=x";
        assertEquals("secret-value", CookieUtils.findCookie(cookies, "ci_c"));
    }

    @Test
    public void cookieFingerprintIsOrderIndependent() {
        assertEquals(
                CookieUtils.fingerprint("b=2; a=1"),
                CookieUtils.fingerprint("a=1; b=2"));
    }

    @Test
    public void urlPolicyRejectsLookalikeCleartextAndCustomPort() {
        assertTrue(SafeUrlPolicy.isAllowedTopLevelUrl(
                "https://gallog.dcinside.com/test/posting"));
        assertFalse(SafeUrlPolicy.isAllowedTopLevelUrl(
                "http://gallog.dcinside.com/test/posting"));
        assertFalse(SafeUrlPolicy.isAllowedTopLevelUrl(
                "https://dcinside.com.evil.example/"));
        assertFalse(SafeUrlPolicy.isAllowedTopLevelUrl(
                "https://gallog.dcinside.com:444/test"));
        assertFalse(SafeUrlPolicy.isAllowedApiUrl("https://www.dcinside.com/"));
    }

    @Test
    public void captchaMarkerInsideNormalListDoesNotBecomeGate() {
        String html = "<ul class='cont_listbox'><li data-no='1'>recaptcha text</li></ul>";
        assertFalse(DcHtmlParser.looksLikeCaptchaGate(html));
        assertTrue(DcHtmlParser.looksLikeCaptchaGate("<div class='g-recaptcha'></div>"));
    }
}
