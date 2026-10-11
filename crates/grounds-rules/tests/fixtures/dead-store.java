class A {
    int m(int x) {
        int a = compute(); // expect
        a = 5;
        int b = 0;
        b = x;
        int c = x * 2;
        c = c + 1;
        String s = null;
        s = "y";
        int d = 1;
        if (x > 0) d = 2;
        return a + b + c + d + s.length();
    }
    void n() {
        int e = compute();
        Runnable r = () -> System.out.println(e);
    }
    int compute() { return 1; }
}
class ContractAsserts {
    @org.jetbrains.annotations.Contract("false -> fail")
    static void check(boolean b) { if (!b) throw new AssertionError(); }
    int m(int x) {
        int y = x + 1;
        check(x > 0);
        return y;
    }
}
interface CleanupService { default void deleteByTenant(String t) { throw new UnsupportedOperationException(); } }
class Overridden {
    CleanupService svc;
    Object find(String t) { return t; }
    void delete(String t) {
        Object tenant = find(t);
        svc.deleteByTenant(t);
        System.out.println(tenant);
    }
}
