require 'json'
require 'time'
require 'yaml'

locale = YAML.safe_load_file(ARGV.fetch(0), aliases: true).fetch('en')
entries = JSON.parse(STDIN.read).fetch('entries')
result = entries.map do |entry|
  record = entry.fetch('record')
  job = record.fetch('job')
  raise 'job payload must be an object' unless job.is_a?(Hash)

  report_type = job['report_type']
  title = if report_type
            locale.dig('reports', report_type, 'title') || report_type
          else
            locale.dig('job', 'types', record.fetch('job_type')) ||
              "Translation missing: en.job.types.#{record.fetch('job_type')}"
          end
  files = entry.fetch('output_files').map do |file|
    "/jobs/#{record.fetch('uri').split('/').last}/file/#{file}"
  end
  filenames = job['filenames'] || [job['filename']].compact
  job_data = files.map do |link|
    filename = filenames.shift
    filename ? "input_file --- #{filename}" : "output_file --- #{link}"
  end
  job.each do |key, value|
    next if %w[jsonmodel_type filenames report_type].include?(key)
    job_data << "#{key} --- #{value}"
  end
  times = %w[time_submitted time_started time_finished].to_h do |field|
    [field, record[field] && Time.parse(record[field]).getlocal.to_s]
  end
  { 'title' => title, 'files' => files, 'job_data' => job_data }.merge(times)
end
STDOUT.write(JSON.generate(result))
