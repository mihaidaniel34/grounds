import org.nolib.Uni;
class A {
    void m(Uni<String> u, Report r) {
        try { work(); } catch (Exception e) { e.printStackTrace(); } // expect
        try { work(); } catch (Exception e) { e.printStackTrace(System.out); }
        // lambda parameter whose type is unknown (library jar missing)
        u.onFailure().recoverWithItem(th -> { th.printStackTrace(); return "x"; }); // expect
        u.onFailure().invoke(f -> f.getCause().printStackTrace()); // expect
        // project class with its own printStackTrace()
        r.printStackTrace();
        new Report().printStackTrace();
    }
    void work() throws Exception {}
}
class Report { void printStackTrace() {} }
