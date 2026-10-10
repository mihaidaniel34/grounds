class Base {
    protected String name;
    private int secret;
}
class Sub extends Base {
    String name; // expect
    int secret;
}
