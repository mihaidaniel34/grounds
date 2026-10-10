class A {
    int m(int i, Integer boxed) {
        int a = Integer.valueOf(i).intValue(); // expect
        long b = new Long(3L).longValue(); // expect
        return boxed.intValue();
    }
}
