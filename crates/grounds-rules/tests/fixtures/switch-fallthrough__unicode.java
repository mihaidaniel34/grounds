class UnicodeCase {
    void f(int i) {
        switch (i) {
            case 0: System.out.println("éxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
            case 1: break; // expect
        }
    }
}
