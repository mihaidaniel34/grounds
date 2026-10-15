package p;
// test support from a library: no sources to follow
public interface Harness {
  default void rewriteRun(String before, String after) {}
  default <T> T exchange(String url, Class<T> type, org.springframework.test.web.servlet.ResultMatcher expected) { return null; }
}
