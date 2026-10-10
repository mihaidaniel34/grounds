class A {
    boolean m(Object o, Class<?> c) {
        if (o.getClass().getName().equals("com.x.Foo")) return true; // expect
        if ("Foo".equals(c.getSimpleName())) return true; // expect
        return o instanceof A;
    }
}
