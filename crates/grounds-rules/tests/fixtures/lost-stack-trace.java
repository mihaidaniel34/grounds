class A {
    void m() {
        try { work(); } catch (java.io.IOException e) { throw new IllegalStateException("failed"); } // expect
        try { work(); } catch (java.io.IOException e) { throw new IllegalStateException(e.getMessage()); } // expect
        try { work(); } catch (java.io.IOException e) { throw new IllegalStateException("failed", e); }
        try { work(); } catch (java.io.IOException e) { RuntimeException r = new RuntimeException("x"); r.initCause(e); throw r; }
        try { work(); } catch (java.io.IOException e) { throw new UncheckedIOException2(e); }
        try { work(); } catch (java.io.IOException e) { Throwable cause = e.getCause(); throw new UncheckedIOException2(cause); }
        try { work(); } catch (java.io.IOException e) { throw new UncheckedIOException2(e.getCause()); }
        try { work(); } catch (java.io.IOException e) { throw new IllegalStateException("failed: " + e.getClass().getSimpleName()); } // expect
        try { work(); } catch (java.io.IOException e) { handle(e); throw new IllegalStateException("unreachable"); }
        try { work(); } catch (java.io.IOException e) { if (isMissing(e)) { throw new IllegalStateException("missing"); } } // expect
        try { work(); } catch (java.io.IOException e) { isMissing(e); throw new IllegalStateException("failed"); } // expect
        try { work(); } catch (java.io.IOException e) { if (debug) { handle(e); throw new IllegalStateException("failed"); } }
        try { work(); } catch (java.io.IOException e) { missing.Lib.rethrowUnchecked(e); throw new IllegalStateException("unreachable"); }
        try { work(); } catch (java.io.IOException e) { if (e.getCause() instanceof java.net.SocketException se) { throw new IllegalStateException("socket", se); } }
        try { work(); } catch (java.io.IOException e) { if ((Object) String.valueOf(e) instanceof String msg) { throw new IllegalStateException(msg); } } // expect
        try { work(); } catch (java.io.IOException e) { throw new IllegalStateException("failed"); } finally { handle(null); } // expect
        try { work(); } catch (java.io.IOException e) { if (debug) handle(e); throw new IllegalStateException("failed"); } // expect
        try { work(); } catch (java.io.IOException e) { String msg = String.valueOf(e); throw new IllegalStateException(msg); } // expect
        try { work(); } catch (missing.RemoteException e) { throw new IllegalStateException(e.getMessage()); } // expect
    }
    void work() throws java.io.IOException {}
    void handle(Exception e) { throw new RuntimeException(e); }
    boolean isMissing(Exception e) { return false; }
    boolean debug;
}
class UncheckedIOException2 extends RuntimeException { UncheckedIOException2(Throwable t) { super(t); } }
class LocalStore {
    void m() {
        try { work(); } catch (java.io.IOException e) { Throwable last = null; last = e; throw new RuntimeException("failed"); } // expect
    }
    void work() throws java.io.IOException {}
}
class AliasCleared {
    void m() {
        Throwable last = null;
        try { work(); } catch (java.io.IOException e) { last = e; last = null; throw new RuntimeException("failed", last); } // expect
    }
    void work() throws java.io.IOException {}
}
class InputErrors {
    // invalid input translated into an error that names the input or carries the parser's message
    Mode mode(String value) {
        try { return Mode.valueOf(value.toUpperCase()); } catch (IllegalArgumentException e) { throw new BadFlag("Not a valid mode: " + value); }
    }
    java.util.regex.Pattern pattern(String regex) {
        try { return java.util.regex.Pattern.compile(regex); } catch (java.util.regex.PatternSyntaxException e) { throw new BadFlag("Pattern syntax error: " + e.getMessage()); }
    }
    long when(String text) {
        try { return java.time.Instant.parse(text).toEpochMilli(); } catch (java.time.format.DateTimeParseException e) { throw new BadFlag("Expected a timestamp but got: " + text); }
    }
    // still lost: the message names neither the input nor the parser's error
    Mode lineMode(String[] values, int line) {
        try { return Mode.valueOf(values[1]); } catch (IllegalArgumentException e) { throw new BadFlag("Error parsing mode in line " + line); } // expect
    }
    // still lost: an I/O failure is not invalid input
    void read(java.io.File f) {
        try { new java.io.FileReader(f).close(); } catch (java.io.IOException e) { throw new BadFlag("Cannot read " + f); } // expect
    }
    // a charset or digest every Java platform supports cannot be missing
    byte[] utf8(String s) {
        try { return s.getBytes("UTF-8"); } catch (java.io.UnsupportedEncodingException e) { throw new AssertionError("UTF-8 is not supported"); }
    }
    java.security.MessageDigest sha1() {
        try { return java.security.MessageDigest.getInstance("SHA-1"); } catch (java.security.NoSuchAlgorithmException e) { throw new IllegalStateException("no SHA-1"); }
    }
    byte[] encoded(String s, String charset) {
        try { return s.getBytes(charset); } catch (java.io.UnsupportedEncodingException e) { throw new IllegalStateException("bad charset"); } // expect
    }
    java.security.MessageDigest digest() {
        try { return java.security.MessageDigest.getInstance("WHIRLPOOL"); } catch (java.security.NoSuchAlgorithmException e) { throw new IllegalStateException("no digest"); } // expect
    }
    // still lost: the second call names its algorithm at run time
    java.security.MessageDigest pair(String algorithm) {
        try { java.security.MessageDigest.getInstance("SHA-256"); return java.security.MessageDigest.getInstance(algorithm); } catch (java.security.NoSuchAlgorithmException e) { throw new IllegalStateException("failed"); } // expect
    }
    // still lost: the literal is not what the call was given
    byte[] encodedLabel(String s, String charset) {
        try { String label = "UTF-8"; return s.getBytes(charset); } catch (java.io.UnsupportedEncodingException e) { throw new IllegalStateException("bad charset"); } // expect
    }
    // still lost: the message names a local the try only read, not the input it parsed
    Mode tagged(String[] values, String tag) {
        try { String seen = tag; return Mode.valueOf(values[0]); } catch (IllegalArgumentException e) { throw new BadFlag("bad mode for " + tag); } // expect
    }
    enum Mode { A, B }
}
class BadFlag extends RuntimeException { BadFlag(String m) { super(m); } }
