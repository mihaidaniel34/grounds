class A {
    void m(int i, String s) {
        Integer a = new Integer(5); // expect
        Boolean b = new Boolean(true); // expect
        Long c = new Long(s); // expect
        Integer ok = Integer.valueOf(i);
    }
}
