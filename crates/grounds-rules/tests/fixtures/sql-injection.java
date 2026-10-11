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
