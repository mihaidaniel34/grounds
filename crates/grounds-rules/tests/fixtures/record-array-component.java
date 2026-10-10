record Packet(byte[] data, int id) { } // expect
record Point(int x, int y) { }
record Fixed(int[] values) {
    @Override public boolean equals(Object o) { return o instanceof Fixed f && java.util.Arrays.equals(values, f.values); }
    @Override public int hashCode() { return java.util.Arrays.hashCode(values); }
}
