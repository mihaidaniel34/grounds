class A {
    void m(int[] a, char[] c, String[] s, StringBuilder sb) {
        String x = a.toString(); // expect
        int h = s.hashCode(); // expect
        String y = "values: " + a; // expect
        System.out.println(s); // expect
        System.out.println(c);
        String z = String.valueOf(a); // expect
        String w = String.valueOf(c);
        sb.append(a); // expect
        sb.append(c);
        String ok = java.util.Arrays.toString(a);
    }
}
