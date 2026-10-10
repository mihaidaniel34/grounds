class A {
    String m(int x) {
        if (x == 1) return "a";
        else if (x == 2) return "b";
        else if (x == 1) return "c"; // expect
        if (next() == 1) { } else if (next() == 1) { }
        return "";
    }
    int next() { return 0; }
}
