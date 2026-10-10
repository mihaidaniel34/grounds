class A {
    void m(int x) {
        switch (x) { // expect
            case 1: break;
            default: break;
        }
        switch (x) { case 1: break; case 2: break; case 3: break; }
    }
}
