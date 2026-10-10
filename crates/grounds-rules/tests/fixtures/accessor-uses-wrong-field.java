class A {
    private String name;
    private String title;
    String getName() { return title; } // expect
    void setName(String n) { this.title = n; } // expect
    String getTitle() { return title; }
    void setTitle(String t) { this.title = t; }
}
