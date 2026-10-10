class A {
    int m(boolean c, int a) {
        if (c) { // expect
            a++;
        } else {
            a++;
        }
        int b = c ? a : a; // expect
        if (a == 1) {
            System.out.println("one");
            a = a * 2;
        } else if (a == 2) { // expect
            System.out.println("one");
            a = a * 2;
        } else if (a == 3) {
            System.out.println("three");
        }
        switch (a) { // expect
            case 1: a++; break;
            default: a++; break;
        }
        return c ? a : b;
    }
}
