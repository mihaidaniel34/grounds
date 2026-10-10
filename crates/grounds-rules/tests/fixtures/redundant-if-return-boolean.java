class A {
    boolean m(int x) {
        if (x > 0) { // expect
            return true;
        } else {
            return false;
        }
    }
    boolean n(int x) {
        if (x > 0) return false; // expect
        return true;
    }
    boolean o(int x) {
        if (x > 0) return true;
        return true;
    }
}
