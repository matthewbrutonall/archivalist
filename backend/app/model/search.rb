require 'net/http'
require 'uri'
require_relative 'search_resolver'

class Search

  def self.search(params, repo_id)
    show_suppressed = !RequestContext.get(:enforce_suppression)
    show_published_only = RequestContext.get(:current_username) === User.PUBLIC_USERNAME

    Log.debug("backend search received params: #{params.inspect}")

    if shadow_sidecar_eligible?(params, repo_id)
      sidecar_result = execute_shadow_sidecar_search(params, repo_id)
      return sidecar_result if sidecar_result
    end

    query = if params[:q]
              Solr::Query.create_keyword_search(params[:q])
            elsif params[:aq] && params[:aq]['query']
              Solr::Query.create_advanced_search(params[:aq], protect_unpublished: show_published_only)
            else
              Solr::Query.create_match_all_query
            end

    query.pagination(params[:page], params[:page_size]).
      set_repo_id(repo_id).
      set_record_types(params[:type]).
      show_suppressed(show_suppressed).
      show_published_only(show_published_only).
      set_excluded_ids(params[:exclude]).
      set_filter(params[:filter]).
      set_filter_queries(params[:filter_query]).
      set_facets(params[:facet], (params[:facet_mincount] || 0)).
      set_sort(params[:sort]).
      set_root_record(params[:root_record]).
      highlighting(params[:hl]).
      set_writer_type( params[:dt] || "json" )

    query.remove_csv_header if ( params[:dt] == "csv" and params[:no_csv_header] )
    query.limit_fields_to(params[:fields]) if params[:fields] && (AppConfig[:limit_csv_fields] || params[:dt] != "csv")

    results = Solr.search(query)

    if params[:resolve]
      # As with the ArchivesSpace API, resolving a field gives a way of
      # returning linked records without having to make multiple queries.
      #
      # In the case of searching, a resolve parameter like:
      #
      #      &resolve[]=repository:id
      #
      # will take the (stored) field value for "repository" and search for
      # that value in the "id" field of other Solr documents.  Any document(s)
      # returned will be inserted into the search response under the key
      # "_resolved_repository".
      #
      # Since you might want to resolve a multi-valued field, we'll use the
      # following format:
      #
      #      "_resolved_myfield": {
      #          "/stored/value/1": [{... matched record 1...}, {... matched record 2...}],
      #          "/stored/value/2": [{... matched record 1...}, {... matched record 2...}]
      #      }
      #
      # To avoid the inlined resolved records being unreasonably large, you can
      # also specify a custom resolver to be used when rendering the record.
      # For example, the query:
      #
      #      &resolve[]=resource:id@compact_resource
      #
      # will use the "compact_resource" resolver to render the inlined resource
      # records.  This is defined by `search_resolver_compact_resource.rb`.  You
      # can define as many of these classes as needed, and they'll be available
      # via the API in this same way.
      resolver = SearchResolver.new(params[:resolve])
      resolver.resolve(results)
    end

    results
  end

  def self.records_for_uris(uris, resolve = [])
    show_suppressed = !RequestContext.get(:enforce_suppression)
    show_published_only = RequestContext.get(:current_username) === User.PUBLIC_USERNAME

    boolean_query = JSONModel.JSONModel(:boolean_query)
                    .from_hash('op' => 'OR',
                               'subqueries' => uris.map {|uri|
                                 field = uri.end_with?('#pui') ? 'id' : 'uri'
                                 JSONModel.JSONModel(:field_query)
                                   .from_hash('field' => field,
                                              'value' => uri,
                                              'literal' => true)
                                   .to_hash
                               })

    query = Solr::Query.create_advanced_search(JSONModel.JSONModel(:advanced_query).from_hash('query' => boolean_query))

    query.pagination(1, uris.length).
          show_suppressed(show_suppressed).
          show_published_only(show_published_only)

    results = Solr.search(query)

    resolver = SearchResolver.new(resolve)
    resolver.resolve(results)

    # Keep the ordering that we were passed in our list of URIs
    results['results'].sort_by! {|result| uris.index(result['uri'])}

    results
  end

  def self.record_type_counts(record_types, for_repo_uri = nil)
    show_suppressed = !RequestContext.get(:enforce_suppression)
    show_published_only = RequestContext.get(:current_username) === User.PUBLIC_USERNAME

    result = {}

    repos_of_interest = if for_repo_uri
                          [for_repo_uri]
                        else
                          Repository.filter(:hidden => 0).select(:id).map do |row|
                            repo_id = row[:id]
                            JSONModel.JSONModel(:repository).uri_for(repo_id)
                          end
                        end

    repos_of_interest.each do |repo_uri|
      result[repo_uri] ||= {}

      record_types.each do |record_type|
        boolean_query = JSONModel.JSONModel(:boolean_query)
                        .from_hash('op' => 'AND',
                                   'subqueries' => [
                                     JSONModel.JSONModel(:boolean_query).from_hash('op' => 'OR',
                                                                                   'subqueries' => [
                                                                                     JSONModel.JSONModel(:field_query)
                                                                                     .from_hash('field' => 'used_within_repository',
                                                                                                'value' => repo_uri,
                                                                                                'literal' => true).to_hash,
                                                                                     JSONModel.JSONModel(:field_query)
                                                                                     .from_hash('field' => 'repository',
                                                                                                'value' => repo_uri,
                                                                                                'literal' => true).to_hash
                                                                                   ]),
                                     JSONModel.JSONModel(:field_query)
                                       .from_hash('field' => 'types',
                                                  'value' => record_type,
                                                  'literal' => true).to_hash,
                                     JSONModel.JSONModel(:field_query)
                                       .from_hash('field' => 'published',
                                                  'value' => 'true',
                                                  'literal' => true).to_hash

                                   ])

        query = Solr::Query.create_advanced_search(JSONModel.JSONModel(:advanced_query).from_hash('query' => boolean_query))
        query.pagination(1, 1).
              show_suppressed(show_suppressed).
              show_published_only(show_published_only)

        hits = Solr.search(query)

        result[repo_uri][record_type] = hits['total_hits']
      end
    end

    if for_repo_uri
      # We're just targeting a single repo
      result.values.first
    else
      result
    end
  end

  def self.search_csv( params, repo_id )
    if use_extended_csv_export?(params)
      extended_csv_export(params, repo_id)
    else
      search_csv_solr(params, repo_id)
    end
  end

  # csv_export_use_solr_writer=true indicates the caller wants raw Solr
  # field names as headers (e.g. the top container browse column picker,
  # see ANW-2791) so skip extended export.
  def self.use_extended_csv_export?(params)
    return false unless AppConfig[:extended_csv_export_enabled]
    return false if params[:csv_export_use_solr_writer].to_s == 'true'

    true
  end

  def self.extended_csv_export(params, repo_id)
    csv_clz = Kernel.const_get(AppConfig[:extended_csv_export_class].intern)

    Enumerator.new do |y|
      csv = csv_clz.new
      begin
        page = 1
        loop do
          hits = search(params.merge(:fields => ['json'], :dt => 'json',
                                     :page => page, :page_size => 200),
                        repo_id)
          break if hits.fetch('results', []).empty?

          hits.fetch('results').each do |result|
            csv << ASUtils.json_parse(result.fetch('json'))
          end
          page += 1
        end

        csv.to_csv do |chunk|
          y << chunk
        end
      ensure
        csv.close
      end
    end
  end

  def self.search_csv_solr( params, repo_id )
    # first let's get a json response with the number of pages
    p = params.dup
    p[:dt] = "json"
    result = search(p, repo_id)
    total_pages = result["last_page"].to_i || 2
    page = 2 # we start on the second page bc the first will have headers

    Enumerator.new do |y|
      # we get page 1 of csv w headers
      y << search(params, repo_id)
      params[:no_csv_header] = true
      while page <= total_pages
        params[:page] = page
        y << search(params, repo_id)
        page +=1
      end
    end
  end

  def self.param_get(params, key)
    params[key] || params[key.to_s]
  end

  def self.shadow_sidecar_eligible?(params, repo_id)
    return false unless repo_id

    return false unless AppConfig.has_key?(:shadow_search_sidecar_enabled) && AppConfig[:shadow_search_sidecar_enabled]

    # params[:shadow_sidecar] is true-like OR "true" OR "1" OR "yes"
    sidecar_flag = param_get(params, :shadow_sidecar)
    return false unless sidecar_flag == true || %w[true 1 yes].include?(sidecar_flag.to_s.strip.downcase)

    # params[:dt] is absent or "json"
    dt = param_get(params, :dt).to_s.strip.downcase
    return false unless dt.empty? || dt == 'json'

    # params[:q] is present and contains a simple keyword string
    q = param_get(params, :q)
    return false unless q.is_a?(String) && !q.strip.empty?

    # params[:root_record] is present and is a resource URI like /repositories/:repo_id/resources/:id matching current repo_id
    root_record = param_get(params, :root_record)
    return false unless root_record.is_a?(String)
    match = root_record.match(%r{\A/repositories/(\d+)/resources/\d+\z})
    return false unless match && match[1] == repo_id.to_s

    # params[:aq], params[:filter], params[:filter_query], params[:facet], params[:resolve], params[:hl], params[:exclude] are absent/empty
    [:aq, :filter, :filter_query, :facet, :resolve, :hl, :exclude].each do |param_key|
      val = param_get(params, param_key)
      next if val.nil?

      if val.respond_to?(:empty?)
        return false unless val.empty?
      else
        return false
      end
    end

    true
  end

  def self.execute_shadow_sidecar_search(params, repo_id)
    base_url = (AppConfig[:shadow_search_sidecar_url] || 'http://127.0.0.1:4570').to_s.chomp('/')
    sidecar_uri = URI.parse("#{base_url}/shadow/search")

    page_val = param_get(params, :page)
    page_size_val = param_get(params, :page_size)

    page = [page_val.to_i, 1].max
    page_size = page_size_val ? page_size_val.to_i : (AppConfig[:default_page_size] || 10).to_i
    page_size = 10 if page_size <= 0

    start = (page - 1) * page_size
    rows = page_size

    query_params = {
      'q' => param_get(params, :q).to_s.strip,
      'resource_uri' => param_get(params, :root_record).to_s.strip,
      'start' => start.to_s,
      'rows' => rows.to_s
    }
    sidecar_uri.query = URI.encode_www_form(query_params)

    timeout = (AppConfig[:shadow_search_sidecar_timeout_seconds] || 1).to_f
    timeout = 1.0 if timeout <= 0

    req = Net::HTTP::Get.new(sidecar_uri.request_uri)

    res = ASHTTP.start_uri(sidecar_uri, :open_timeout => timeout, :read_timeout => timeout) do |http|
      http.request(req)
    end

    return nil unless res && res.code.to_s =~ /\A2\d\d\z/

    data = ASUtils.json_parse(res.body)
    return nil unless data.is_a?(Hash) && data['num_found'] && data['docs'].is_a?(Array)

    transform_sidecar_search_response(data, page, page_size, start)
  rescue StandardError => e
    Log.warn("Shadow search sidecar request failed, falling back to Solr search: #{e.class} - #{e.message}")
    nil
  end

  def self.transform_sidecar_search_response(data, page, page_size, start)
    num_found = data['num_found'].to_i
    last_page = (num_found / page_size.to_f).ceil

    results = (data['docs'] || []).map do |doc|
      next unless doc.is_a?(Hash)

      uri = doc['id']
      title = doc['title'] || doc['display_string']
      display_string = doc['display_string'] || doc['title']

      item = {
        'uri' => uri,
        'title' => title,
        'primary_type' => 'archival_object'
      }

      item['resource'] = doc['resource_uri'] if doc['resource_uri']
      item['display_string'] = doc['display_string'] if doc.key?('display_string')
      item['parent_uri'] = doc['parent_uri'] if doc.key?('parent_uri')
      item['ancestor_uris'] = doc['ancestor_uris'] if doc.key?('ancestor_uris')
      item['ancestor_levels'] = doc['ancestor_levels'] if doc.key?('ancestor_levels')
      item['date_expressions'] = doc['date_expressions'] if doc.key?('date_expressions')

      json_record = {
        'jsonmodel_type' => 'archival_object',
        'uri' => uri,
        'title' => title,
        'display_string' => display_string
      }
      json_record['resource'] = { 'ref' => doc['resource_uri'] } if doc['resource_uri']
      item['json'] = json_record

      item
    end.compact

    offset_first = num_found.zero? ? 0 : start + 1
    offset_last = num_found.zero? ? 0 : [start + page_size, num_found].min

    {
      'page_size' => page_size,
      'first_page' => 1,
      'last_page' => last_page,
      'this_page' => page,
      'offset_first' => offset_first,
      'offset_last' => offset_last,
      'total_hits' => num_found,
      'results' => results
    }
  end

end
