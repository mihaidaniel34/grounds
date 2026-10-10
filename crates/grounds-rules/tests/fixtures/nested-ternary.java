class A {
    String m(int x) {
        String a = x > 0 ? "pos" : x < 0 ? "neg" : "zero"; // expect
        return x > 0 ? "a" : "b";
    }
}
