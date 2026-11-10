// grounds: test, grounds: no-findings
class StreamTest {
    void writesAfterCloseFail(java.io.File f) throws java.io.IOException {
        java.io.OutputStream out = new java.io.FileOutputStream(f);
        out.close();
        // a test checking what a closed stream does
        try { out.write(1); } catch (java.io.IOException expected) { }
    }
}
