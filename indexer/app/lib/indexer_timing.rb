class IndexerTiming
  def initialize
    @metrics = {}
    @mutex = Mutex.new
  end

  def add(metric, val)
    @mutex.synchronize do
      @metrics[metric] ||= 0
      @metrics[metric] += val.to_i
    end
  end

  def to_s
    @mutex.synchronize do
      subtotal = @metrics.values.inject(0) {|a, b| a + b}

      if @total
        # If we have a total, report any difference between the total and the
        # numbers we have.
        @metrics[:other] ||= 0
        @metrics[:other] += (@total - subtotal).to_i
      else
        # Otherwise, just tally up our numbers to determine the total.
        @total = subtotal
      end

      "#{@total.to_i} ms (#{@metrics.map {|k, v| "#{k}: #{v}"}.join('; ')})"
    end
  end

  def total=(ms)
    @mutex.synchronize do
      @total = ms
    end
  end

  def time_block(metric)
    start_time = Time.now
    begin
      yield
    ensure
      add(metric, ((Time.now.to_f * 1000) - (start_time.to_f * 1000)))
    end
  end
end
