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
