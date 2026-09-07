package demo;

import demo.model.Order;

public class OrderService {
  private final Repo repo;

  public OrderService(Repo repo) {
    this.repo = repo;
  }

  public void save(Order o) {
    repo.persist(o);
    this.log();
    Order fresh = new Order();
  }

  void log() {}
}
