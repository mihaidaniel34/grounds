package testkit;
import static org.junit.jupiter.api.Assertions.assertTrue;
// a base class in a test source set: its helpers are followed
public abstract class WebHarness {
  protected <T> T post(String url, Object body, Class<T> type, int status) { return read(perform(url, body), status, type); }
  protected String fetch(String url) { return perform(url, null).toString(); }
  private <T> T read(Object response, int status, Class<T> type) { checkStatus(response, status); return null; }
  private void checkStatus(Object response, int status) { assertTrue(response != null && status > 0); }
  protected Object perform(String url, Object body) { return url; }
  protected void reset() {}
}
