class A {
    void m(StringBuilder sb, String a, String b) {
        sb.append(a + b); // expect
        sb.append("x" + "y");
        sb.append(a).append(b);
    }
}
