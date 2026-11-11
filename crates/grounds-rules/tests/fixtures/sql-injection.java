import java.sql.*;
import org.springframework.jdbc.core.JdbcTemplate;
class A {
    void m(Connection c, Statement st, JdbcTemplate jdbc, String name) throws SQLException {
        st.executeQuery("select * from users where name = '" + name + "'"); // expect
        c.prepareStatement(String.format("delete from %s", name)); // expect
        String q = "select * from t where id = " + name;
        jdbc.queryForList(q); // expect
        c.prepareStatement("select * from users where name = ?");
        jdbc.update("update t set a = ?", name);
    }
}
class ConstantSql {
    void a(java.sql.Connection c) throws java.sql.SQLException {
        StringBuilder sb = new StringBuilder();
        sb.append("SELECT id FROM users");
        c.prepareStatement(sb.toString());
    }
    void b(java.sql.Connection c, String in) throws java.sql.SQLException {
        String sql = "SELECT * FROM t WHERE x = '" + in + "'";
        sql = "SELECT 1";
        c.prepareStatement(sql);
    }
    void c(java.sql.Connection c) throws java.sql.SQLException {
        c.prepareStatement(String.format("SELECT 1 FROM dual"));
    }
    void d(java.sql.Connection c, String in) throws java.sql.SQLException {
        StringBuilder sb = new StringBuilder("SELECT * FROM t WHERE x = ");
        sb.append(in);
        c.prepareStatement(sb.toString()); // expect
    }
}
class ConditionalSql {
    void a(java.sql.Statement st, String user, boolean reset) throws Exception {
        String q = "SELECT * FROM t WHERE u = '" + user + "'";
        if (reset) q = "SELECT 1";
        st.executeQuery(q); // expect
    }
    void b(java.sql.Statement st, String user) throws Exception {
        st.executeQuery(user.concat(";")); // expect
    }
}
class CompoundSql {
    void m(java.sql.Statement st, String user) throws Exception {
        String q = "SELECT * FROM users WHERE name='" + user + "'";
        q += ";";
        st.executeQuery(q); // expect
    }
}
// identifiers from fields and constants, values bound: nothing the caller sends reaches the text
class IdentifiersFromFields {
    static final String SCHEMA = System.getProperty("schema");
    private final String table;
    private final String idColumn;
    private final java.util.List<String> indexes;
    IdentifiersFromFields(String table, String idColumn, java.util.List<String> indexes) {
        this.table = table;
        this.idColumn = idColumn;
        this.indexes = indexes;
    }
    String name() { return table; }
    void bound(java.sql.Connection c, String id) throws java.sql.SQLException {
        java.sql.PreparedStatement ps = c.prepareStatement("DELETE FROM " + table + " WHERE " + idColumn + " = ?");
        ps.setString(1, id);
        String sql = String.format("UPDATE %s.%s SET seen = ? WHERE %s = ?", SCHEMA, this.table, idColumn);
        c.prepareStatement(sql);
    }
    void ddl(java.sql.Statement st, int dimension) throws java.sql.SQLException {
        st.executeUpdate(String.format("TRUNCATE TABLE %s", table));
        st.executeUpdate("DROP TABLE IF EXISTS " + name());
        String index = table + "_idx";
        st.executeUpdate("CREATE INDEX " + index + " ON " + table + " (v vector(" + dimension + "))");
        for (String col : indexes) {
            st.executeUpdate("CREATE INDEX ON " + table + " (" + col.trim() + ")");
        }
        indexes.forEach(col -> {
            try {
                st.executeUpdate("CREATE INDEX ON " + table + " (" + col + ")");
            } catch (java.sql.SQLException e) {
                throw new RuntimeException(e);
            }
        });
    }
    void numbers(java.sql.Statement st, int limit, Long id) throws java.sql.SQLException {
        st.executeQuery("SELECT * FROM " + table + " WHERE id = " + id + " LIMIT " + limit);
    }
    void builder(java.sql.Connection c) throws java.sql.SQLException {
        StringBuilder sb = new StringBuilder("INSERT INTO ").append(table).append(" VALUES (");
        sb.append("?").append(")");
        c.prepareStatement(sb.toString());
    }
    // a parameter still injects, in a value or beside a field
    void value(java.sql.Statement st, String name) throws java.sql.SQLException {
        st.executeQuery("SELECT * FROM " + table + " WHERE name = '" + name + "'"); // expect
    }
    void derived(java.sql.Statement st, String name) throws java.sql.SQLException {
        String quoted = "'" + name.trim() + "'";
        String sql = String.format("SELECT * FROM %s WHERE name = %s", table, quoted);
        st.executeQuery(sql); // expect
    }
    void appended(java.sql.Connection c, String name) throws java.sql.SQLException {
        StringBuilder sb = new StringBuilder("SELECT * FROM ").append(table);
        sb.append(" WHERE name = '").append(name).append("'");
        c.prepareStatement(sb.toString()); // expect
    }
    void each(java.sql.Statement st, java.util.List<String> names) throws java.sql.SQLException {
        for (String n : names) {
            st.executeUpdate("DELETE FROM " + table + " WHERE name = '" + n + "'"); // expect
        }
    }
    void streamed(java.sql.Statement st, java.util.List<String> names) throws java.sql.SQLException {
        String in = names.stream().map(n -> "'" + n + "'").collect(java.util.stream.Collectors.joining(","));
        st.executeUpdate("DELETE FROM " + table + " WHERE name IN (" + in + ")"); // expect
    }
    // DDL takes no bind parameters, but a name from the caller still reaches the statement
    void createIndex(java.sql.Statement st, String table, String column) throws java.sql.SQLException {
        String sql = String.format("create index if not exists %s_%s on %s (%s)", table, column, table, column);
        st.executeUpdate(sql); // expect
        st.execute("DROP TABLE IF EXISTS " + table); // expect
    }
    // the caller names the table and column, the value is bound
    void named(java.sql.Connection c, String owner, String table, String column, String pref) throws java.sql.SQLException {
        String query = String.format("select rowid, to_text(t.%s, json(?)) from %s.%s t", column, owner, table);
        java.sql.PreparedStatement ps = c.prepareStatement(query);
        ps.setString(1, pref);
        c.prepareStatement("UPDATE " + table + " SET seen = ? WHERE id = ?");
    }
    // an apostrophe in a comment opens no literal
    void commented(java.sql.Connection c, String column, String name) throws java.sql.SQLException {
        if (!java.util.Set.of("id", "name").contains(column)) throw new IllegalArgumentException();
        c.prepareStatement("SELECT /* don't expose details */ t." + column + " FROM t WHERE id = ?");
        c.prepareStatement("SELECT -- don't expose details\n t." + column + " FROM t WHERE id = ?");
        c.prepareStatement("SELECT 'it''s', t." + column + " FROM t WHERE id = ?");
        c.prepareStatement("SELECT * FROM t /* it's */ WHERE id = ? AND name = '" + name + "'"); // expect
        c.prepareStatement("SELECT * FROM t WHERE id = ? /* " + name + " */"); // expect
    }
    // a value spliced in beside the placeholders
    void mixed(java.sql.Connection c, String table, String name) throws java.sql.SQLException {
        c.prepareStatement("SELECT * FROM " + table + " WHERE id = ? AND name = '" + name + "'"); // expect
        c.prepareStatement(String.format("SELECT * FROM %2$s WHERE id = ? AND name = '%1$s'", name, table)); // expect
        c.prepareStatement("SELECT * FROM t WHERE id = ? ORDER BY " + name); // expect
        c.prepareStatement("SELECT * FROM t WHERE id = ? AND host = 'www." + name + "'"); // expect
    }
    void getter(java.sql.Statement st, javax.servlet.http.HttpServletRequest req) throws java.sql.SQLException {
        st.executeQuery("SELECT * FROM " + table + " WHERE name = '" + req.getParameter("name") + "'"); // expect
    }
}
// a table or column name beside placeholders: reported when it visibly comes from the request
class RequestIdentifiers {
    void getter(java.sql.Connection c, javax.servlet.http.HttpServletRequest req) throws java.sql.SQLException {
        c.prepareStatement("SELECT * FROM " + req.getParameter("table") + " WHERE id = ?"); // expect
        String column = req.getParameter("column");
        c.prepareStatement("SELECT t." + column + " FROM t WHERE id = ?"); // expect
    }
    void bound(java.sql.Connection c, @org.springframework.web.bind.annotation.RequestParam String table) throws java.sql.SQLException {
        c.prepareStatement("UPDATE " + table + " SET seen = ? WHERE id = ?"); // expect
    }
    @org.springframework.web.bind.annotation.GetMapping("/rows")
    void handler(java.sql.Connection c, String table) throws java.sql.SQLException {
        c.prepareStatement("DELETE FROM " + table + " WHERE id = ?"); // expect
    }
    // the request held in a field
    javax.servlet.http.HttpServletRequest request;
    void injected(java.sql.Connection c, java.sql.Statement st) throws java.sql.SQLException {
        c.prepareStatement("SELECT * FROM " + request.getParameter("table") + " WHERE id = ?"); // expect
        st.executeQuery("SELECT * FROM t WHERE name = '" + request.getParameter("name") + "'"); // expect
    }
    // the caller chose the name: nothing here says it is the request's
    void helper(java.sql.Connection c, String table, UnknownRequest other, Settings settings) throws java.sql.SQLException {
        c.prepareStatement("UPDATE " + table + " SET seen = ? WHERE id = ?");
        c.prepareStatement("SELECT * FROM " + other.getParameter("table") + " WHERE id = ?");
        c.prepareStatement("SELECT * FROM " + settings.getParameter("table") + " WHERE id = ?");
    }
}
// a getter of the same name on a class that is no request
class Settings {
    String getParameter(String key) { return key; }
}
