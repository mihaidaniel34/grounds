class A {
    String m(int i, Object o) {
        String a = "n=" + String.valueOf(i); // expect
        String b = String.valueOf(i);
        String c = String.valueOf(i) + 1;
        return String.valueOf(o) + "x"; // expect
    }
}
