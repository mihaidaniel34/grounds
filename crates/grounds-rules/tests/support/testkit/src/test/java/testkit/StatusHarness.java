package testkit;
// a base class whose helper delegates to one inherited from another file
public abstract class StatusHarness extends WebHarness {
  protected void expectCreated(String url) { send(url, 201); }
  // an overload of an inherited helper
  protected <T> T post(String url, Object body, Class<T> type) { return post(url, body, type, 200); }
  private void send(String url, int status) { post(url, "{}", String.class, status); }
}
