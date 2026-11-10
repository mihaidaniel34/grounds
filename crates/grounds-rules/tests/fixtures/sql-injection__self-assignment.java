import java.sql.Statement;

// `q = q;` reads the value from before it. This used to recurse until the
// stack overflowed and took the whole run down (sonar-java's test sources).
class SelfAssignment {
    void built(Statement st, String table) throws Exception {
        String q = "select * from " + table;
        q = q;
        st.execute(q); // expect
    }

    void constant(Statement st) throws Exception {
        String q = "select * from t";
        q = q;
        st.execute(q);
    }

    void twice(Statement st, String table, boolean b) throws Exception {
        String q = "select 1";
        if (b) {
            q = "select * from " + table;
        }
        q = (q);
        q = q;
        st.execute(q); // expect
    }
}
