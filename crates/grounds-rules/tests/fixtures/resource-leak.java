import java.io.*;
import java.nio.file.*;
class A {
    void m(File f, Path p) throws IOException {
        InputStream a = new FileInputStream(f); // expect
        a.read();
        InputStream b = new FileInputStream(f);
        try { b.read(); } finally { b.close(); }
        try (InputStream c = new FileInputStream(f)) { c.read(); }
        BufferedReader r = Files.newBufferedReader(p); // expect
        r.readLine();
        ByteArrayOutputStream bos = new ByteArrayOutputStream();
        bos.write(1);
    }
    InputStream open(File f) throws IOException {
        InputStream in = new FileInputStream(f);
        return in;
    }
    Reader wrap(File f) throws IOException {
        InputStream in = new FileInputStream(f);
        return new InputStreamReader(in);
    }
}
class Idioms {
    void closeInTry(File file) throws IOException {
        FileInputStream fis = new FileInputStream(file);
        try { fis.close(); } catch (IOException e) { System.err.println("close failed: " + e); }
    }
    void nullGuarded(File file) throws IOException {
        FileInputStream fis = null;
        try {
            fis = new FileInputStream(file);
            fis.read();
        } finally {
            if (fis != null) { fis.close(); }
        }
    }
    String jdbc(javax.sql.DataSource ds, String title) throws java.sql.SQLException {
        try (java.sql.Connection c = ds.getConnection()) {
            java.sql.PreparedStatement ps = c.prepareStatement("select title from books where title = ?");
            ps.setString(1, title);
            java.sql.ResultSet rs = ps.executeQuery();
            return rs.next() ? rs.getString("title") : null;
        }
    }
    void jdbcLeak(java.sql.Connection c) throws java.sql.SQLException {
        java.sql.Statement st = c.createStatement(); // expect
        st.execute("select 1");
    }
}
class JdbcReassigned {
    void m(java.sql.Statement external) throws Exception {
        try (java.sql.Connection c = java.sql.DriverManager.getConnection("jdbc:test")) {
            java.sql.Statement st = c.createStatement();
            st = external;
            java.sql.ResultSet rs = st.executeQuery("SELECT 1"); // expect
            rs.next();
        }
    }
}
// In-memory streams: OutputStream.close() and InputStream.close() do nothing,
// so a project subclass that never overrides close() holds no resource.
class FastBytesOutputStream extends OutputStream {
    private byte[] buf = new byte[16];
    private int count;
    private boolean closed;
    @Override public void close() { this.closed = true; }
    @Override public void write(int b) { if (count == buf.length) buf = java.util.Arrays.copyOf(buf, count * 2); buf[count++] = (byte) b; }
    byte[] toByteArray() { return java.util.Arrays.copyOf(buf, count); }
}
class TaggedBytesOutputStream extends FastBytesOutputStream {
    String tag = "t";
}
class EmptyInput extends InputStream {
    @Override public int read() { return -1; }
}
class SpoolStream extends OutputStream {
    private final OutputStream out;
    SpoolStream(File f) throws IOException { out = new FileOutputStream(f); }
    @Override public void write(int b) throws IOException { out.write(b); }
    @Override public void close() throws IOException { out.close(); }
}
class TeeStream extends OutputStream {
    private final OutputStream target;
    TeeStream(OutputStream target) { this.target = target; }
    @Override public void write(int b) throws IOException { target.write(b); }
}
class LogFileStream extends FilterOutputStream {
    LogFileStream(File f) throws IOException { super(new FileOutputStream(f)); }
}
class ClosingFlagStream extends OutputStream {
    private final StringBuilder log = new StringBuilder();
    @Override public void write(int b) { log.append((char) b); }
    @Override public void close() { System.out.println(log); }
}
class Handle implements Closeable {
    @Override public void close() { }
    void use() { }
}
class InMemoryStreams {
    private static class CountingOutputStream extends OutputStream {
        long count;
        @Override public void write(int b) { count++; }
    }
    Runnable render() {
        return () -> {
            FastBytesOutputStream bos = new FastBytesOutputStream();
            bos.write(1);
            System.out.println(bos.toByteArray().length);
        };
    }
    long size(byte[] data) throws IOException {
        CountingOutputStream counter = new CountingOutputStream();
        counter.write(data);
        return counter.count;
    }
    int tagged() {
        TaggedBytesOutputStream t = new TaggedBytesOutputStream();
        t.write(2);
        return t.toByteArray().length;
    }
    int empty() throws IOException {
        EmptyInput in = new EmptyInput();
        return in.read();
    }
    void overridesClose(File f) throws IOException {
        SpoolStream s = new SpoolStream(f); // expect
        s.write(1);
    }
    void wrapsStream(File f) throws IOException {
        TeeStream t = new TeeStream(new FileOutputStream(f)); // expect
        t.write(1);
    }
    void filter(File f) throws IOException {
        LogFileStream c = new LogFileStream(f); // expect
        c.write(1);
    }
    void closeDoesWork() {
        ClosingFlagStream s = new ClosingFlagStream(); // expect
        s.write(1);
    }
    void projectCloseable() {
        Handle h = new Handle(); // expect
        h.use();
    }
}
// Wrappers around a resource the method does not own: closing them would
// close the caller's response or stream.
class ResponseAdapter implements Closeable {
    ResponseAdapter(javax.servlet.http.HttpServletResponse response) { }
    void flush() { }
    @Override public void close() { }
}
class JakartaResponseAdapter implements Closeable {
    JakartaResponseAdapter(jakarta.servlet.http.HttpServletResponse response) { }
    void flush() { }
    @Override public void close() { }
}
class Guard implements Closeable {
    Guard(Closeable c) { }
    void use() { }
    @Override public void close() { }
}
class Session implements Closeable {
    Session(Closeable parent, File log) { }
    void use() { }
    @Override public void close() { }
}
class Holder {
    InputStream stream;
}
class ConfiguredGuard implements Closeable {
    private final int size;
    ConfiguredGuard(Closeable c) throws IOException {
        try (InputStream in = new FileInputStream("guard.properties")) { size = in.read(); }
    }
    void use() { }
    @Override public void close() { }
}
class SpooledGuard implements Closeable {
    private final OutputStream spool;
    SpooledGuard(Closeable c) throws IOException { spool = java.nio.file.Files.newOutputStream(java.nio.file.Paths.get("spool")); }
    void use() { }
    @Override public void close() throws IOException { spool.close(); }
}
class ConnectedGuard implements Closeable {
    private java.sql.Connection connection;
    ConnectedGuard(Closeable c) throws java.sql.SQLException {
        try (java.sql.Statement s = (this.connection = java.sql.DriverManager.getConnection("jdbc:test")).createStatement()) { s.execute("select 1"); }
    }
    void use() { }
    @Override public void close() throws IOException {
        try { connection.close(); } catch (java.sql.SQLException e) { throw new IOException(e); }
    }
}
class LinesGuard implements Closeable {
    private final java.util.stream.Stream<String> lines;
    LinesGuard(Closeable c) throws IOException { lines = Files.lines(Paths.get("guard.txt")); }
    void use() { }
    @Override public void close() { lines.close(); }
}
interface HolderFactory {
    Holder makeHolder();
}
class AuditedGuard implements Closeable {
    private final OutputStream audit;
    AuditedGuard(Closeable c) throws IOException { audit = new FileOutputStream("audit.log"); }
    void use() { }
    @Override public void close() throws IOException { audit.close(); }
}
class LoggedGuard implements Closeable {
    private final Writer log = new FileWriter("guard.log");
    LoggedGuard(Closeable c) throws IOException { }
    void use() { }
    @Override public void close() throws IOException { log.close(); }
}
class Shared {
    static Handle handle;
}
interface NativeRequest {
    <T> T getNativeResponse(Class<T> type);
}
class Handlers {
    private javax.servlet.http.HttpServletResponse current;
    private Handle shared;
    void param(javax.servlet.http.HttpServletResponse response) {
        ResponseAdapter out = new ResponseAdapter(response);
        out.flush();
    }
    void derived(NativeRequest request) {
        javax.servlet.http.HttpServletResponse servletResponse = request.getNativeResponse(javax.servlet.http.HttpServletResponse.class);
        ResponseAdapter out = new ResponseAdapter(servletResponse);
        out.flush();
    }
    void jakarta(jakarta.servlet.http.HttpServletResponse response) {
        JakartaResponseAdapter out = new JakartaResponseAdapter(response);
        out.flush();
    }
    void inline(NativeRequest request) {
        ResponseAdapter out = new ResponseAdapter(request.getNativeResponse(javax.servlet.http.HttpServletResponse.class));
        out.flush();
    }
    void field() {
        ResponseAdapter out = new ResponseAdapter(this.current);
        out.flush();
        Guard g = new Guard(shared);
        g.use();
    }
    void staticField() {
        Guard g = new Guard(Shared.handle);
        g.use();
    }
    class Inner {
        void outerField() {
            Guard g = new Guard(Handlers.this.shared);
            g.use();
        }
    }
    void closeableParam(Handle h, InputStream in) {
        Guard g = new Guard(h);
        g.use();
        Guard s = new Guard(in);
        s.use();
    }
    void opensFromParam(java.net.URL url) throws IOException {
        Guard g = new Guard(url.openStream()); // expect
        g.use();
    }
    void opensOwnFile(Closeable parent, File log) {
        Session s = new Session(parent, log); // expect
        s.use();
    }
    void freshHolder() {
        Guard g = new Guard(new Holder().stream); // expect
        g.use();
    }
    void reassignedParam(InputStream in, File f) throws IOException {
        in = new FileInputStream(f);
        Guard g = new Guard(in); // expect
        g.use();
    }
    void fieldOfFreshHolder(HolderFactory factory) {
        Guard g = new Guard(factory.makeHolder().stream); // expect
        g.use();
    }
    void wrapperOpensInCtor(Handle h) throws IOException {
        AuditedGuard g = new AuditedGuard(h); // expect
        g.use();
    }
    void wrapperOpensInField(Handle h) throws IOException {
        LoggedGuard g = new LoggedGuard(h); // expect
        g.use();
    }
    void wrapperOpensQualified(Handle h) throws IOException {
        SpooledGuard g = new SpooledGuard(h); // expect
        g.use();
    }
    void wrapperKeepsConnection(Handle h) throws java.sql.SQLException {
        ConnectedGuard g = new ConnectedGuard(h); // expect
        g.use();
    }
    void wrapperKeepsLines(Handle h) throws IOException {
        LinesGuard g = new LinesGuard(h); // expect
        g.use();
    }
    void wrapperReadsConfig(Handle h) throws IOException {
        ConfiguredGuard g = new ConfiguredGuard(h);
        g.use();
    }
    void ownsInner() {
        Guard g = new Guard(new Handle()); // expect
        g.use();
    }
    void ownsLocal() {
        Handle h = new Handle();
        Guard g = new Guard(h); // expect
        g.use();
    }
    void ownsFile(File f) throws IOException {
        Guard g = new Guard(new FileInputStream(f)); // expect
        g.use();
    }
}
