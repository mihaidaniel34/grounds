class A {
    boolean m(String a, String b) {
        if (a.toLowerCase().equals(b.toLowerCase())) return true; // expect
        if (a.toUpperCase().equals("X")) return true; // expect
        return a.equalsIgnoreCase(b);
    }
}
