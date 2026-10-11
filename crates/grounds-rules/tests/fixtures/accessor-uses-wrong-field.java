class A {
    private String name;
    private String title;
    String getName() { return title; } // expect
    void setName(String n) { this.title = n; } // expect
    String getTitle() { return title; }
    void setTitle(String t) { this.title = t; }
}
class Elem {
    private Compensation compensation;
    private boolean isCompensation;
    public boolean isCompensation() { return isCompensation; }
    public void setCompensation(boolean isCompensation) { this.isCompensation = isCompensation; }
    static class Compensation {}
}
