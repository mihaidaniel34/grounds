class A {
    void m(String name, int n, double d, Object[] arr) {
        String a = String.format("%s and %s", name); // expect
        String b = String.format("%d", name); // expect
        String c = String.format("%q", name); // expect
        System.out.printf("%s%n", name, n); // expect
        String e = "%s=%d".formatted(name); // expect
        String f = String.format("%f", n); // expect
        String ok = String.format("%s=%d (%.2f) %%", name, n, d);
        String ok2 = String.format("%2$s %1$s", name, name);
        String ok3 = String.format("%s %s", arr);
        String ok4 = String.format(java.util.Locale.ROOT, "%d", n);
    }
}
