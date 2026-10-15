require 'spec_helper'

describe Search do

  let(:repo) {
    create(:repo)
  }

  let(:aq_notes) {
    build(:json_advanced_query,
          query: build(:json_field_query,
                       field: 'notes',
                       value: 'foobar',
                       literal: false,
                       comparator: 'contains',
                       negated: false))
  }

  let(:params) {
    {
      aq: aq_notes,
      "q.op": "AND",
      fields: ["notes"],
      page: 1,
      page_size: 1
    }
  }

  let(:params_csv) {
    {
      aq: aq_notes,
      "q.op": "AND",
      fields: ["notes"],
      page: 1,
      page_size: 1,
      dt: "csv"
    }
  }

  it "can build a url for solr to search the notes field" do
    allow(Solr).to receive(:search) { |solr_query|
      solr_url = solr_query.to_solr_url
      params = URI.decode_www_form(solr_url.query)
      expect(params.find {|param| param.first == 'q'}[1]).to eq "notes:(foobar)"
      expect(params.find {|param| param.first == 'qf'}[1]).to match /fullrecord$/
    }

    Search.search(params, repo.id)
  end

  context "when AppConfig[:limit_csv_fields] = false" do

    before do
      AppConfig[:limit_csv_fields] = false
    end

    it "includes fl for json data type query" do
      allow(Solr).to receive(:search) { |solr_query|
        solr_url = solr_query.to_solr_url
        params = URI.decode_www_form(solr_url.query)
        expect(params.find {|param| param.first == 'q'}[1]).to eq "notes:(foobar)"
        expect(params.find {|param| param.first == 'qf'}[1]).to match /fullrecord$/
        expect(params.find {|param| param.first == 'fl'}[1]).to eq "notes"
      }
      Search.search(params, repo.id)
    end

    it "excludes fl for csv data type query" do
      allow(Solr).to receive(:search) { |solr_query|
        solr_url = solr_query.to_solr_url
        params = URI.decode_www_form(solr_url.query)
        expect(params.find {|param| param.first == 'q'}[1]).to eq "notes:(foobar)"
        expect(params.find {|param| param.first == 'qf'}[1]).to match /fullrecord$/
        expect(params.find {|param| param.first == 'fl'}).to be_nil
      }
      Search.search(params_csv, repo.id)
    end
  end

  context "when AppConfig[:limit_csv_fields] = true" do

    before do
      AppConfig[:limit_csv_fields] = true
    end

    it "includes fl for json data type query" do
      allow(Solr).to receive(:search) { |solr_query|
        solr_url = solr_query.to_solr_url
        params = URI.decode_www_form(solr_url.query)
        expect(params.find {|param| param.first == 'q'}[1]).to eq "notes:(foobar)"
        expect(params.find {|param| param.first == 'qf'}[1]).to match /fullrecord$/
        expect(params.find {|param| param.first == 'fl'}[1]).to eq "notes"
      }
      Search.search(params, repo.id)
    end

    it "include fl for csv data type query" do
      allow(Solr).to receive(:search) { |solr_query|
        solr_url = solr_query.to_solr_url
        params = URI.decode_www_form(solr_url.query)
        expect(params.find {|param| param.first == 'q'}[1]).to eq "notes:(foobar)"
        expect(params.find {|param| param.first == 'qf'}[1]).to match /fullrecord$/
        expect(params.find {|param| param.first == 'fl'}[1]).to eq "notes"
      }
      Search.search(params_csv, repo.id)
    end
  end

  it "can build a url for solr to search the notes field while protecting unpublished data from the public user" do
    allow(Solr).to receive(:search) { |solr_query|
      solr_url = solr_query.to_solr_url
      params = URI.decode_www_form(solr_url.query)
      expect(params.find {|param| param.first == 'q'}[1]).to eq "notes_published:(foobar)"
      expect(params.find {|param| param.first == 'qf'}[1]).to match /fullrecord_published/
    }
    as_test_user(User.PUBLIC_USERNAME) do
      Search.search(params, repo.id)
    end
  end

  context "Search result CSV export" do

    let(:mock_search_results) {
      {
        "results" => [
          {
            "json" => build(:json_resource).to_json
          }
        ]
      }
    }

    let(:mock_empty_results) {
      {
        "results" => []
      }
    }

    it "uses the extended CSV export when enabled" do
      allow(AppConfig).to receive(:[]).and_call_original
      allow(AppConfig).to receive(:[]).with(:extended_csv_export_enabled).and_return(true)

      allow(Search).to receive(:search).with(hash_including(:page => 1), repo.id).and_return(mock_search_results)
      allow(Search).to receive(:search).with(hash_including(:page => 2), repo.id).and_return(mock_empty_results)

      csv = ""
      Search.search_csv({:page => 1, :page_size => 10}, repo.id).each do |csv_chunk|
        csv << csv_chunk
      end

      expect(csv).to include("dates::0::")
    end

    it "uses the Solr CSV writer when csv_export_use_solr_writer is requested (ANW-2791)" do
      allow(AppConfig).to receive(:[]).and_call_original
      allow(AppConfig).to receive(:[]).with(:extended_csv_export_enabled).and_return(true)

      expect(Search).to receive(:search_csv_solr).with(hash_including(:fields => ['barcode_u_sstr']), repo.id)
      expect(Search).not_to receive(:extended_csv_export)

      Search.search_csv({:fields => ['barcode_u_sstr'], :csv_export_use_solr_writer => 'true', :dt => 'csv'}, repo.id)
    end

    it "uses the extended CSV export for field-limited requests without csv_export_use_solr_writer (ANW-2924)" do
      allow(AppConfig).to receive(:[]).and_call_original
      allow(AppConfig).to receive(:[]).with(:extended_csv_export_enabled).and_return(true)

      allow(Search).to receive(:search).with(hash_including(:page => 1), repo.id).and_return(mock_search_results)
      allow(Search).to receive(:search).with(hash_including(:page => 2), repo.id).and_return(mock_empty_results)

      expect(Search).not_to receive(:search_csv_solr)

      csv = ""
      Search.search_csv({:fields => ['title'], :page => 1, :page_size => 10}, repo.id).each do |csv_chunk|
        csv << csv_chunk
      end

      expect(csv).to include("dates::0::")
    end

  end

  context "Rust shadow search sidecar fast path" do
    let(:sidecar_params) {
      {
        :q => "letters",
        :root_record => "/repositories/#{repo.id}/resources/1",
        :shadow_sidecar => "true",
        :page => 1,
        :page_size => 10
      }
    }

    let(:mock_sidecar_json) {
      {
        "num_found" => 1,
        "start" => 0,
        "rows" => 10,
        "q" => "letters",
        "resource_uri" => "/repositories/#{repo.id}/resources/1",
        "docs" => [
          {
            "id" => "/repositories/#{repo.id}/archival_objects/42",
            "title" => "Letters to John",
            "display_string" => "Letters to John, 1920",
            "resource_uri" => "/repositories/#{repo.id}/resources/1",
            "parent_uri" => "/repositories/#{repo.id}/archival_objects/40",
            "ancestor_uris" => ["/repositories/#{repo.id}/resources/1", "/repositories/#{repo.id}/archival_objects/40"],
            "ancestor_levels" => ["collection", "series"],
            "date_expressions" => ["1920"]
          }
        ]
      }.to_json
    }

    let(:mock_solr_response) {
      {
        "total_hits" => 1,
        "results" => [
          {
            "uri" => "/repositories/#{repo.id}/archival_objects/42",
            "title" => "From Solr",
            "primary_type" => "archival_object"
          }
        ]
      }
    }

    before do
      allow(AppConfig).to receive(:[]).and_call_original
      allow(AppConfig).to receive(:has_key?).and_call_original
      allow(AppConfig).to receive(:has_key?).with(:shadow_search_sidecar_enabled).and_return(true)
      allow(AppConfig).to receive(:[]).with(:shadow_search_sidecar_enabled).and_return(false)
      allow(AppConfig).to receive(:[]).with(:shadow_search_sidecar_url).and_return("http://127.0.0.1:4570")
      allow(AppConfig).to receive(:[]).with(:shadow_search_sidecar_timeout_seconds).and_return(1)
      allow(AppConfig).to receive(:[]).with(:default_page_size).and_return(10)
    end

    it "falls through to Solr.search when shadow sidecar is disabled in config" do
      allow(AppConfig).to receive(:[]).with(:shadow_search_sidecar_enabled).and_return(false)

      expect(Solr).to receive(:search).and_return(mock_solr_response)
      expect(ASHTTP).not_to receive(:start_uri)

      res = Search.search(sidecar_params, repo.id)
      expect(res).to eq(mock_solr_response)
    end

    it "falls back to Solr.search and does not raise when AppConfig key :shadow_search_sidecar_enabled is missing" do
      allow(AppConfig).to receive(:has_key?).with(:shadow_search_sidecar_enabled).and_return(false)
      allow(AppConfig).to receive(:[]).with(:shadow_search_sidecar_enabled).and_raise("No value set for config parameter: shadow_search_sidecar_enabled")

      expect(Solr).to receive(:search).and_return(mock_solr_response)
      expect(ASHTTP).not_to receive(:start_uri)

      res = nil
      expect {
        res = Search.search(sidecar_params, repo.id)
      }.not_to raise_error

      expect(res).to eq(mock_solr_response)
    end

    it "falls through to Solr.search when query shape has unsupported params" do
      allow(AppConfig).to receive(:[]).with(:shadow_search_sidecar_enabled).and_return(true)

      # 1. dt is csv
      expect(Solr).to receive(:search).and_return(mock_solr_response)
      res = Search.search(sidecar_params.merge(:dt => "csv"), repo.id)
      expect(res).to eq(mock_solr_response)

      # 2. repo_id is nil
      expect(Solr).to receive(:search).and_return(mock_solr_response)
      res = Search.search(sidecar_params, nil)
      expect(res).to eq(mock_solr_response)

      # 3. shadow_sidecar flag is absent/false
      expect(Solr).to receive(:search).and_return(mock_solr_response)
      res = Search.search(sidecar_params.merge(:shadow_sidecar => "false"), repo.id)
      expect(res).to eq(mock_solr_response)

      # 4. root_record is invalid format (not a resource URI)
      expect(Solr).to receive(:search).and_return(mock_solr_response)
      res = Search.search(sidecar_params.merge(:root_record => "/repositories/#{repo.id}/archival_objects/1"), repo.id)
      expect(res).to eq(mock_solr_response)

      # 4b. root_record repo does not match repo_id
      expect(Solr).to receive(:search).and_return(mock_solr_response)
      res = Search.search(sidecar_params.merge(:root_record => "/repositories/#{repo.id.to_i + 2}/resources/1"), repo.id)
      expect(res).to eq(mock_solr_response)

      # 5. aq is present
      expect(Solr).to receive(:search).and_return(mock_solr_response)
      res = Search.search(sidecar_params.merge(:aq => aq_notes), repo.id)
      expect(res).to eq(mock_solr_response)

      # 6. filter is present
      expect(Solr).to receive(:search).and_return(mock_solr_response)
      res = Search.search(sidecar_params.merge(:filter => {"query" => "something"}), repo.id)
      expect(res).to eq(mock_solr_response)

      # 7. facet is present
      expect(Solr).to receive(:search).and_return(mock_solr_response)
      res = Search.search(sidecar_params.merge(:facet => ["primary_type"]), repo.id)
      expect(res).to eq(mock_solr_response)

      # 8. resolve is present
      expect(Solr).to receive(:search).and_return(mock_solr_response)
      res = Search.search(sidecar_params.merge(:resolve => ["repository:id"]), repo.id)
      expect(res).to eq(mock_solr_response)
    end

    it "falls back to Solr.search and does not call ASHTTP.start_uri when root_record repository does not match repo_id" do
      allow(AppConfig).to receive(:[]).with(:shadow_search_sidecar_enabled).and_return(true)

      expect(Solr).to receive(:search).and_return(mock_solr_response)
      expect(ASHTTP).not_to receive(:start_uri)

      mismatched_root = "/repositories/#{repo.id.to_i + 2}/resources/1"
      res = Search.search(sidecar_params.merge(:root_record => mismatched_root), repo.id)
      expect(res).to eq(mock_solr_response)
    end

    it "calls the sidecar and returns compatible search hash when params match supported shape" do
      allow(AppConfig).to receive(:[]).with(:shadow_search_sidecar_enabled).and_return(true)

      mock_response = double("Net::HTTPResponse", :code => "200", :body => mock_sidecar_json)
      mock_http = double("Net::HTTP")
      allow(mock_http).to receive(:request).and_return(mock_response)

      captured_uri = nil
      allow(ASHTTP).to receive(:start_uri) do |uri, opts, &block|
        captured_uri = uri
        block.call(mock_http)
      end

      expect(Solr).not_to receive(:search)

      res = Search.search(sidecar_params, repo.id)

      expect(captured_uri).not_to be_nil
      expect(captured_uri.path).to eq("/shadow/search")
      query_items = URI.decode_www_form(captured_uri.query).to_h
      expect(query_items["q"]).to eq("letters")
      expect(query_items["resource_uri"]).to eq("/repositories/#{repo.id}/resources/1")
      expect(query_items["start"]).to eq("0")
      expect(query_items["rows"]).to eq("10")

      expect(res["total_hits"]).to eq(1)
      expect(res["first_page"]).to eq(1)
      expect(res["last_page"]).to eq(1)
      expect(res["this_page"]).to eq(1)
      expect(res["offset_first"]).to eq(1)
      expect(res["offset_last"]).to eq(1)
      expect(res["page_size"]).to eq(10)

      expect(res["results"].length).to eq(1)
      doc = res["results"].first
      expect(doc["uri"]).to eq("/repositories/#{repo.id}/archival_objects/42")
      expect(doc["title"]).to eq("Letters to John")
      expect(doc["primary_type"]).to eq("archival_object")
      expect(doc["resource"]).to eq("/repositories/#{repo.id}/resources/1")
      expect(doc["display_string"]).to eq("Letters to John, 1920")
      expect(doc["parent_uri"]).to eq("/repositories/#{repo.id}/archival_objects/40")
      expect(doc["ancestor_uris"]).to eq(["/repositories/#{repo.id}/resources/1", "/repositories/#{repo.id}/archival_objects/40"])
      expect(doc["ancestor_levels"]).to eq(["collection", "series"])
      expect(doc["date_expressions"]).to eq(["1920"])
      expect(doc["json"]).to eq({
        "jsonmodel_type" => "archival_object",
        "uri" => "/repositories/#{repo.id}/archival_objects/42",
        "title" => "Letters to John",
        "display_string" => "Letters to John, 1920",
        "resource" => { "ref" => "/repositories/#{repo.id}/resources/1" }
      })
    end

    it "calls the sidecar and returns compatible search hash when params use string keys" do
      allow(AppConfig).to receive(:[]).with(:shadow_search_sidecar_enabled).and_return(true)

      mock_response = double("Net::HTTPResponse", :code => "200", :body => mock_sidecar_json)
      mock_http = double("Net::HTTP")
      allow(mock_http).to receive(:request).and_return(mock_response)

      captured_uri = nil
      allow(ASHTTP).to receive(:start_uri) do |uri, opts, &block|
        captured_uri = uri
        block.call(mock_http)
      end

      expect(Solr).not_to receive(:search)

      string_params = {
        "q" => "letters",
        "root_record" => "/repositories/#{repo.id}/resources/1",
        "shadow_sidecar" => "true",
        "page" => "2",
        "page_size" => "5"
      }

      res = Search.search(string_params, repo.id)

      expect(captured_uri).not_to be_nil
      expect(captured_uri.path).to eq("/shadow/search")
      query_items = URI.decode_www_form(captured_uri.query).to_h
      expect(query_items["q"]).to eq("letters")
      expect(query_items["resource_uri"]).to eq("/repositories/#{repo.id}/resources/1")
      expect(query_items["start"]).to eq("5")
      expect(query_items["rows"]).to eq("5")

      expect(res["total_hits"]).to eq(1)
      expect(res["this_page"]).to eq(2)
      expect(res["page_size"]).to eq(5)
    end

    it "falls back to Solr.search when unsupported params are provided as string keys" do
      allow(AppConfig).to receive(:[]).with(:shadow_search_sidecar_enabled).and_return(true)

      base_string_params = {
        "q" => "letters",
        "root_record" => "/repositories/#{repo.id}/resources/1",
        "shadow_sidecar" => "true"
      }

      # 1. string-keyed "facet"
      expect(Solr).to receive(:search).and_return(mock_solr_response)
      res = Search.search(base_string_params.merge("facet" => ["primary_type"]), repo.id)
      expect(res).to eq(mock_solr_response)

      # 2. string-keyed "filter"
      expect(Solr).to receive(:search).and_return(mock_solr_response)
      res = Search.search(base_string_params.merge("filter" => {"query" => "something"}), repo.id)
      expect(res).to eq(mock_solr_response)

      # 3. string-keyed "dt" => "csv"
      expect(Solr).to receive(:search).and_return(mock_solr_response)
      res = Search.search(base_string_params.merge("dt" => "csv"), repo.id)
      expect(res).to eq(mock_solr_response)

      # 4. string-keyed "resolve"
      expect(Solr).to receive(:search).and_return(mock_solr_response)
      res = Search.search(base_string_params.merge("resolve" => ["repository:id"]), repo.id)
      expect(res).to eq(mock_solr_response)
    end

    it "falls back to Solr.search when sidecar returns non-2xx status" do
      allow(AppConfig).to receive(:[]).with(:shadow_search_sidecar_enabled).and_return(true)

      mock_response = double("Net::HTTPResponse", :code => "500", :body => '{"error":"internal error"}')
      mock_http = double("Net::HTTP")
      allow(mock_http).to receive(:request).and_return(mock_response)
      allow(ASHTTP).to receive(:start_uri) do |uri, opts, &block|
        block.call(mock_http)
      end

      expect(Solr).to receive(:search).and_return(mock_solr_response)

      res = Search.search(sidecar_params, repo.id)
      expect(res).to eq(mock_solr_response)
    end

    it "falls back to Solr.search when sidecar returns invalid JSON" do
      allow(AppConfig).to receive(:[]).with(:shadow_search_sidecar_enabled).and_return(true)

      mock_response = double("Net::HTTPResponse", :code => "200", :body => "NOT VALID JSON")
      mock_http = double("Net::HTTP")
      allow(mock_http).to receive(:request).and_return(mock_response)
      allow(ASHTTP).to receive(:start_uri) do |uri, opts, &block|
        block.call(mock_http)
      end

      expect(Solr).to receive(:search).and_return(mock_solr_response)

      res = Search.search(sidecar_params, repo.id)
      expect(res).to eq(mock_solr_response)
    end

    it "falls back to Solr.search when sidecar request times out or raises an HTTP error" do
      allow(AppConfig).to receive(:[]).with(:shadow_search_sidecar_enabled).and_return(true)

      allow(ASHTTP).to receive(:start_uri).and_raise(Timeout::Error.new("connection timed out"))

      expect(Solr).to receive(:search).and_return(mock_solr_response)

      res = Search.search(sidecar_params, repo.id)
      expect(res).to eq(mock_solr_response)
    end
  end
end
