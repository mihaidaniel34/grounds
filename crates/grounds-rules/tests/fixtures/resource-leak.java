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
