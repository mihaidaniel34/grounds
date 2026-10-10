// grounds: no-findings
class Sample {
    String[] toArray() { return new String[]{"x"}; }
    String[] copy() { return (String[]) toArray(); }
}
