package p;
// main code: its precondition checks are not test assertions
public class Inventory {
  public void add(String item) { checkNotEmpty(item); }
  private void checkNotEmpty(String s) { if (s.isEmpty()) throw new IllegalArgumentException(); }
}
