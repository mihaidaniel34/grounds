class A {
    String m() {
        try { return read(); } catch (Exception e) { return null; } // expect
    }
    String n() {
        try { return read(); } catch (Exception e) {
            // absent is fine here
            return null;
        }
    }
    String read() throws Exception { return ""; }
}
