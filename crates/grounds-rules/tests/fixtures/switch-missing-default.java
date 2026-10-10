enum E { A, B }
class X {
    void m(int x, E e) {
        switch (x) { // expect
            case 1: break;
            case 2: break;
        }
        switch (x) { case 1: break; default: break; }
        switch (e) { case A: break; }
    }
}
