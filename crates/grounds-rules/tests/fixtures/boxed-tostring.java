class A {
    String m(int i, Integer boxed) {
        String a = Integer.valueOf(i).toString(); // expect
        String b = new Integer(i).toString(); // expect
        String c = boxed.toString();
        return Integer.toString(i);
    }
}
