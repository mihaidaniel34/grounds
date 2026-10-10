class A {
    void m(Object o) {
        if (o instanceof String s) { } // expect
        if (o instanceof Integer i) { System.out.println(i); }
    }
}
