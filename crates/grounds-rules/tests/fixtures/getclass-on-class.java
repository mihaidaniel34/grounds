class A {
    void m(Class<?> c, Object o) {
        Object a = c.getClass(); // expect
        Object b = String.class.getClass(); // expect
        Object d = o.getClass();
    }
}
