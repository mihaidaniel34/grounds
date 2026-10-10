class A {
    boolean m(boolean f, Boolean boxed, boolean g) {
        if (f == true) return true; // expect
        if (false != f) return true; // expect
        boolean x = g ? true : false; // expect
        boolean y = g ? false : true; // expect
        boolean z = f && true; // expect
        if (boxed == true) return false;
        return f;
    }
}
